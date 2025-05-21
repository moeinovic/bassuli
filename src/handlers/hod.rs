use std::borrow::Cow;
use anyhow::anyhow;
use rust_i18n::t;
use teloxide::Bot;
use teloxide::macros::BotCommands;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{LinkPreviewOptions, Message, UserId};
use crate::{config, metrics, repo};
use crate::config::DickOfDaySelectionMode;
use crate::domain::LanguageCode;
use crate::handlers::{FromRefs, HandlerResult, reply_html, utils};
use crate::handlers::utils::Incrementor;

const HOD_ALREADY_CHOSEN_SQL_CODE: &str = "GD0E2"; // Using the same SQL code as DOD

#[derive(BotCommands, Clone)]
#[command(rename_rule = "snake_case")]
pub enum HemoroidOfDayCommands {
    #[command(description = "hod")]
    HemoroidOfDay,
    Hod,
}

pub async fn hod_cmd_handler(bot: Bot, msg: Message,
                             cfg: config::AppConfig, repos: repo::Repositories, incr: Incrementor) -> HandlerResult {
    metrics::CMD_DOD_COUNTER.chat.inc(); // Reusing the DOD counter 
    let from = msg.from.as_ref().ok_or(anyhow!("unexpected absence of a FROM field"))?;
    let chat_id = msg.chat.id.into();
    let from_refs = FromRefs(from, &chat_id);
    let answer = hemoroid_of_day_impl(cfg, &repos, incr, from_refs).await?;
    reply_html(bot, &msg, answer)
        .link_preview_options(disabled_link_preview())
        .await?;
    Ok(())
}

pub(crate) async fn hemoroid_of_day_impl(cfg: config::AppConfig, repos: &repo::Repositories, incr: Incrementor,
                                     from_refs: FromRefs<'_>) -> anyhow::Result<String> {
    let (from, chat_id) = (from_refs.0, from_refs.1);
    let lang_code = LanguageCode::from_user(from);
    let winner = match cfg.features.dod_selection_mode {
        DickOfDaySelectionMode::WEIGHTS => {
            repos.users.get_random_active_member_with_poor_in_priority(&chat_id.kind()).await?
        },
        DickOfDaySelectionMode::EXCLUSION if cfg.dod_rich_exclusion_ratio.is_some() => {
            let rich_exclusion_ratio = cfg.dod_rich_exclusion_ratio.unwrap();
            repos.users.get_random_active_poor_member(&chat_id.kind(), rich_exclusion_ratio).await?
        },
        _ => repos.users.get_random_active_member(&chat_id.kind()).await?
    };
    let answer = match winner {
        Some(winner) => {
            let improvement = incr.dod_increment(from.id, chat_id.kind()).await;
            let hod_result = repos.hemoroids.set_hod_winner(chat_id, UserId(winner.uid as u64), improvement.total).await;
            let main_part = match hod_result {
                Ok(Some(repo::TreatmentResult{ new_protrusion_level, pos_in_top })) => {
                    let answer = t!("commands.hod.result", locale = &lang_code,
                        uid = winner.uid, name = winner.name.escaped(), improvement = improvement.total, level = new_protrusion_level);
                    let perks_part = improvement.perks_part_of_answer(&lang_code);
                    if let Some(pos) = pos_in_top {
                        let position = t!("commands.hod.position", locale = &lang_code, pos = pos);
                        format!("{answer}\n{position}{perks_part}")
                    } else {
                        format!("{answer}{perks_part}")
                    }
                },
                Ok(None) => {
                    log::error!("there was an attempt to set a non-existent hemorrhoid as a winner (UserID={}, ChatId={})",
                        winner.uid, chat_id);
                    t!("commands.hod.no_candidates", locale = &lang_code).to_string()
                }
                Err(e) => {
                    match e.downcast::<sqlx::Error>()? {
                        sqlx::Error::Database(e)
                        if e.code() == Some(Cow::Borrowed(HOD_ALREADY_CHOSEN_SQL_CODE)) => {
                            t!("commands.hod.already_chosen", locale = &lang_code, name = e.message()).to_string()
                        }
                        e => Err(e)?
                    }
                }
            };
            let time_left_part = utils::date::get_time_till_next_day_string(&lang_code);
            format!("{main_part}{time_left_part}")
        },
        None => t!("commands.hod.no_candidates", locale = &lang_code).to_string()
    };
    let announcement = repos.announcements.get_new(&chat_id.kind(), &lang_code).await?
        .map(|announcement| format!("\n\n<i>{announcement}</i>"))
        .unwrap_or_default();
    Ok(format!("{answer}{announcement}"))
}

fn disabled_link_preview() -> LinkPreviewOptions {
    LinkPreviewOptions::builder()
        .is_disabled(true)
        .build()
}
