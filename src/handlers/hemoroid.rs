use std::future::IntoFuture;

use anyhow::{anyhow, Context};
use chrono::{Datelike, Utc};
use futures::future::join;
use futures::TryFutureExt;
use rand::{Rng, thread_rng};
use rust_i18n::t;
use teloxide::Bot;
use teloxide::macros::BotCommands;
use teloxide::requests::Requester;
use teloxide::types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message, ParseMode, ReplyMarkup, User, UserId};

use page::{InvalidPage, Page};

use crate::{config, metrics, repo};
use crate::domain::{LanguageCode, Username};
use crate::handlers::{HandlerResult, reply_html, utils};
use crate::handlers::utils::{callbacks, page};
use crate::repo::{ChatIdPartiality, UID};

const TOMORROW_SQL_CODE: &str = "GD0E1";
const CALLBACK_PREFIX_TOP_PAGE: &str = "top:page:";
const CALLBACK_PREFIX_WORST_PAGE: &str = "worst:page:";

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum HemoroidCommands {
    #[command(description = "shrink")]
    Shrink,
    #[command(description = "stats")]
    Level,
    #[command(description = "top")]
    Top,
    #[command(description = "worst")]
    Worst,
    #[command(description = "clench")]
    Clench,
    #[command(description = "tip")]
    Tip,
}

pub async fn hemoroid_cmd_handler(bot: Bot, msg: Message, cmd: HemoroidCommands,
                              repos: repo::Repositories, incr: utils::Incrementor,
                              config: config::AppConfig) -> HandlerResult {
    let from = msg.from.as_ref().ok_or(anyhow!("unexpected absence of a FROM field"))?;
    let chat_id = msg.chat.id.into();
    let from_refs = FromRefs(from, &chat_id);
    
    match cmd {
        HemoroidCommands::Shrink => {
            metrics::CMD_GROW_COUNTER.chat.inc();
            let answer = shrink_impl(&repos, incr, from_refs).await?;
            reply_html(bot, &msg, answer)
        },
        HemoroidCommands::Level => {
            metrics::CMD_STATS_COUNTER.chat.inc();
            let level_info = level_impl(&repos, from_refs).await?;
            reply_html(bot, &msg, level_info)
        },
        HemoroidCommands::Top => {
            metrics::CMD_TOP_COUNTER.chat.inc();
            let top = top_impl(&repos, &config, from_refs, Page::first()).await?;
            let mut request = reply_html(bot, &msg, top.lines);
            if top.has_more_pages && config.features.top_unlimited {
                let keyboard = ReplyMarkup::InlineKeyboard(build_pagination_keyboard(Page::first(), top.has_more_pages, CALLBACK_PREFIX_TOP_PAGE));
                request.reply_markup.replace(keyboard);
            }
            request
        },
        HemoroidCommands::Worst => {
            metrics::CMD_TOP_COUNTER.chat.inc();
            let worst = worst_impl(&repos, &config, from_refs, Page::first()).await?;
            let mut request = reply_html(bot, &msg, worst.lines);
            if worst.has_more_pages && config.features.top_unlimited {
                let keyboard = ReplyMarkup::InlineKeyboard(build_pagination_keyboard(Page::first(), worst.has_more_pages, CALLBACK_PREFIX_WORST_PAGE));
                request.reply_markup.replace(keyboard);
            }
            request
        },
        HemoroidCommands::Clench => {
            let answer = clench_impl(&from_refs.0.id);
            reply_html(bot, &msg, answer)
        },
        HemoroidCommands::Tip => {
            let tip = get_random_tip(&LanguageCode::from_user(from));
            reply_html(bot, &msg, tip)
        },
    }.await.context(format!("failed for {msg:?}"))?;
    Ok(())
}

pub struct FromRefs<'a>(pub &'a User, pub &'a ChatIdPartiality);

pub(crate) async fn level_impl(repos: &repo::Repositories, from_refs: FromRefs<'_>) -> anyhow::Result<String> {
    let (from, chat_id) = (from_refs.0, from_refs.1);
    let lang_code = LanguageCode::from_user(from);
    
    if let Some(hemoroid) = repos.hemoroids.fetch_hemoroid(from.id, chat_id.kind()).await? {
        let position_str = hemoroid.position.map_or("".to_string(), |pos| {
            format!("\n{}", t!("commands.level.position", locale = &lang_code, pos = pos))
        });
        
        Ok(t!("commands.level.stats", 
            locale = &lang_code, 
            level = hemoroid.protrusion_level,
            pos = hemoroid.position.unwrap_or(0)
        ).to_string() + &position_str)
    } else {
        Ok(t!("commands.level.not_found", locale = &lang_code).to_string())
    }
}

pub(crate) async fn shrink_impl(repos: &repo::Repositories, incr: utils::Incrementor, from_refs: FromRefs<'_>) -> anyhow::Result<String> {
    let (from, chat_id) = (from_refs.0, from_refs.1);
    let name = utils::get_full_name(from);
    let user = repos.users.create_or_update(from.id, &name).await?;
    let days_since_registration = (Utc::now() - user.created_at).num_days() as u32;
    
    let mut rng = thread_rng();
    let shrink_chance = 0.7; // 70% chance to shrink
    
    // Generate a random event: positive = shrink (good), negative = swell (bad)
    let is_shrink = rng.gen_bool(shrink_chance);
    
    // Generate random change amount
    let change_amount = if is_shrink {
        // Shrink by 0.1 to 1.0cm (negative value = shrink)
        -(rng.gen_range(10..101) as i32) / 10
    } else {
        // Swell by 0.1 to 1.5cm (positive value = swell)
        (rng.gen_range(10..151) as i32) / 10
    };
    
    let treatment_result = repos.hemoroids.create_or_shrink(from.id, chat_id, change_amount).await;
    let lang_code = LanguageCode::from_user(from);

    let main_part = match treatment_result {
        Ok(repo::TreatmentResult { new_protrusion_level, pos_in_top }) => {
            let event_key = if change_amount.is_negative() { "shrunk" } else { "swelled" };
            let event_template = format!("commands.shrink.direction.{event_key}");
            let event = t!(&event_template, locale = &lang_code);
            
            let answer = t!("commands.shrink.result", locale = &lang_code,
                event = event, 
                change = change_amount.abs(), 
                level = new_protrusion_level);
            
            if let Some(pos) = pos_in_top {
                let position = t!("commands.shrink.position", locale = &lang_code, pos = pos);
                format!("{answer}\n{position}")
            } else {
                answer.to_string()
            }
        },
        Err(e) => {
            let db_err = e.downcast::<sqlx::Error>()?;
            if let sqlx::Error::Database(e) = db_err {
                e.code()
                    .filter(|c| c == TOMORROW_SQL_CODE)
                    .map(|_| t!("commands.shrink.tomorrow", locale = &lang_code).to_string())
                    .ok_or(anyhow!(e))?
            } else {
                Err(db_err)?
            }
        }
    };
    let time_left_part = utils::date::get_time_till_next_day_string(&lang_code);
    Ok(format!("{main_part}{time_left_part}"))
}

pub(crate) struct Top {
    pub lines: String,
    pub(crate) has_more_pages: bool,
}

impl Top {
    fn from(s: impl ToString) -> Self {
        Self {
            lines: s.to_string(),
            has_more_pages: false,
        }
    }

    fn with_more_pages(s: impl ToString) -> Self {
        Self {
            lines: s.to_string(),
            has_more_pages: true,
        }
    }
}

pub(crate) async fn top_impl(repos: &repo::Repositories, config: &config::AppConfig, from_refs: FromRefs<'_>,
                             page: Page) -> anyhow::Result<Top> {
    let (from, chat_id) = (from_refs.0, from_refs.1.kind());
    let lang_code = LanguageCode::from_user(from);
    let top_limit = config.top_limit as u32;
    let offset = page * top_limit;
    let query_limit = config.top_limit + 1; // fetch +1 row to know whether more rows exist or not
    
    let hemoroids = repos.hemoroids.get_top(&chat_id, offset, query_limit).await?;
    let has_more_pages = hemoroids.len() as u32 > top_limit;
    
    let lines = hemoroids.into_iter()
        .take(config.top_limit as usize)
        .enumerate()
        .map(|(i, h)| {
            let escaped_name = Username::new(h.owner_name).escaped();
            let name = if from.id == <UID as Into<UserId>>::into(h.owner_uid) {
                format!("<u>{escaped_name}</u>")
            } else {
                escaped_name
            };
            let can_shrink = Utc::now().num_days_from_ce() > h.treated_at.num_days_from_ce();
            let pos = h.position.unwrap_or((i+1) as i64);
            let mut line = t!("commands.top.line", 
                locale = &lang_code,
                n = pos, 
                name = name, 
                level = h.protrusion_level).to_string();
            if can_shrink {
                line.push_str(" [+]")
            };
            line
        })
        .collect::<Vec<String>>();

    let res = if lines.is_empty() {
        Top::from(t!("commands.top.empty", locale = &lang_code))
    } else {
        let title = t!("commands.top.title", locale = &lang_code);
        let ending = t!("commands.top.ending", locale = &lang_code);
        let text = format!("{}\n\n{}\n\n{}", title, lines.join("\n"), ending);
        if has_more_pages {
            Top::with_more_pages(text)
        } else {
            Top::from(text)
        }
    };
    Ok(res)
}

pub(crate) async fn worst_impl(repos: &repo::Repositories, config: &config::AppConfig, from_refs: FromRefs<'_>,
                              page: Page) -> anyhow::Result<Top> {
    let (from, chat_id) = (from_refs.0, from_refs.1.kind());
    let lang_code = LanguageCode::from_user(from);
    let top_limit = config.top_limit as u32;
    let offset = page * top_limit;
    let query_limit = config.top_limit + 1; // fetch +1 row to know whether more rows exist or not
    
    let hemoroids = repos.hemoroids.get_worst(&chat_id, offset, query_limit).await?;
    let has_more_pages = hemoroids.len() as u32 > top_limit;
    
    let lines = hemoroids.into_iter()
        .take(config.top_limit as usize)
        .enumerate()
        .map(|(i, h)| {
            let escaped_name = Username::new(h.owner_name).escaped();
            let name = if from.id == <UID as Into<UserId>>::into(h.owner_uid) {
                format!("<u>{escaped_name}</u>")
            } else {
                escaped_name
            };
            let can_shrink = Utc::now().num_days_from_ce() > h.treated_at.num_days_from_ce();
            let pos = h.position.unwrap_or((i+1) as i64);
            let mut line = t!("commands.worst.line", 
                locale = &lang_code,
                n = pos, 
                name = name, 
                level = h.protrusion_level).to_string();
            if can_shrink {
                line.push_str(" [+]")
            };
            line
        })
        .collect::<Vec<String>>();

    let res = if lines.is_empty() {
        Top::from(t!("commands.worst.empty", locale = &lang_code))
    } else {
        let title = t!("commands.worst.title", locale = &lang_code);
        let ending = t!("commands.worst.ending", locale = &lang_code);
        let text = format!("{}\n\n{}\n\n{}", title, lines.join("\n"), ending);
        if has_more_pages {
            Top::with_more_pages(text)
        } else {
            Top::from(text)
        }
    };
    Ok(res)
}

fn build_pagination_keyboard(page: Page, has_more_pages: bool, prefix: &str) -> InlineKeyboardMarkup {
    let mut buttons = Vec::new();
    if page.0 > 0 {
        buttons.push(InlineKeyboardButton::callback("◀️", format!("{}{}", prefix, page.previous())));
    }
    if has_more_pages {
        buttons.push(InlineKeyboardButton::callback("▶️", format!("{}{}", prefix, page.next())));
    }
    InlineKeyboardMarkup::new(vec![buttons])
}

pub fn page_callback_filter(query: CallbackQuery) -> bool {
    query.data
        .as_ref()
        .filter(|d| d.starts_with(CALLBACK_PREFIX_TOP_PAGE) || d.starts_with(CALLBACK_PREFIX_WORST_PAGE))
        .is_some()
}

pub async fn page_callback_handler(bot: Bot, q: CallbackQuery,
                                  config: config::AppConfig, repos: repo::Repositories) -> HandlerResult {
    let edit_msg_req_params = callbacks::get_params_for_message_edit(&q)?;
    if !config.features.top_unlimited {
        return answer_callback_feature_disabled(bot, &q, edit_msg_req_params).await
    }

    let (page, prefix) = if let Some(data) = q.data.as_ref() {
        if data.starts_with(CALLBACK_PREFIX_TOP_PAGE) {
            (data.strip_prefix(CALLBACK_PREFIX_TOP_PAGE)
                .map(str::to_owned)
                .ok_or(InvalidPage::for_value(data, "invalid top prefix"))
                .and_then(|r| r.parse()
                    .map_err(|e| InvalidPage::for_value(&r, e)))
                .map(Page)
                .map_err(|e| anyhow!(e))?, 
             CALLBACK_PREFIX_TOP_PAGE)
        } else if data.starts_with(CALLBACK_PREFIX_WORST_PAGE) {
            (data.strip_prefix(CALLBACK_PREFIX_WORST_PAGE)
                .map(str::to_owned)
                .ok_or(InvalidPage::for_value(data, "invalid worst prefix"))
                .and_then(|r| r.parse()
                    .map_err(|e| InvalidPage::for_value(&r, e)))
                .map(Page)
                .map_err(|e| anyhow!(e))?,
             CALLBACK_PREFIX_WORST_PAGE)
        } else {
            return Err(anyhow!("Unknown callback data prefix").into());
        }
    } else {
        return Err(anyhow!("No callback data").into());
    };

    let chat_id_kind = edit_msg_req_params.clone().into();
    let chat_id_partiality = ChatIdPartiality::Specific(chat_id_kind);
    let from_refs = FromRefs(&q.from, &chat_id_partiality);
    
    let top = if prefix == CALLBACK_PREFIX_TOP_PAGE {
        top_impl(&repos, &config, from_refs, page).await?
    } else {
        worst_impl(&repos, &config, from_refs, page).await?
    };

    let keyboard = build_pagination_keyboard(page, top.has_more_pages, prefix);
    let (answer_callback_query_result, edit_message_result) = match &edit_msg_req_params {
        callbacks::EditMessageReqParamsKind::Chat(chat_id, message_id) => {
            let mut edit_message_text_req = bot.edit_message_text(*chat_id, *message_id, top.lines);
            edit_message_text_req.parse_mode.replace(ParseMode::Html);
            edit_message_text_req.reply_markup.replace(keyboard);
            join(
                bot.answer_callback_query(&q.id).into_future(),
                edit_message_text_req.into_future()
            ).await
        }
        callbacks::EditMessageReqParamsKind::Inline(inline_message_id) => {
            let mut edit_message_text_req = bot.edit_message_text_inline(inline_message_id, top.lines);
            edit_message_text_req.parse_mode.replace(ParseMode::Html);
            edit_message_text_req.reply_markup.replace(keyboard);
            join(
                bot.answer_callback_query(&q.id).into_future(),
                edit_message_text_req.into_future()
            ).await
        }
    };
    answer_callback_query_result.context("couldn't answer the callback query")?;
    edit_message_result.context("couldn't edit message text")?;

    Ok(())
}

async fn answer_callback_feature_disabled<'a>(bot: Bot, callback_query: &'a CallbackQuery,
                                             request_params: callbacks::EditMessageReqParamsKind<'a>) -> HandlerResult {
    let lang_code = LanguageCode::from_user(&callback_query.from);
    let text = t!("errors.feature_disabled", locale = &lang_code);
    match request_params {
        callbacks::EditMessageReqParamsKind::Chat(chat_id, message_id) => {
            bot.answer_callback_query(&callback_query.id)
                .text(text)
                .show_alert(true)
                .await
                .context("couldn't answer the callback query")?;
            bot.edit_message_reply_markup(chat_id, message_id)
                .reply_markup(InlineKeyboardMarkup::new(vec![]))
                .await
                .context("couldn't remove inline buttons")?;
        }
        callbacks::EditMessageReqParamsKind::Inline(inline_message_id) => {
            bot.answer_callback_query(&callback_query.id)
                .text(text)
                .show_alert(true)
                .await
                .context("couldn't answer the callback query")?;
            bot.edit_message_reply_markup_inline(inline_message_id)
                .reply_markup(InlineKeyboardMarkup::new(vec![]))
                .await
                .context("couldn't remove inline buttons")?;
        }
    }
    Ok(())
}

fn clench_impl(user_id: &UserId) -> String {
    let mut rng = thread_rng();
    let success_chance = 0.3; // 30% chance to reduce battle damage
    let success = rng.gen_bool(success_chance);
    
    if success {
        format!("✅ <b>Clench successful!</b> You've activated your pelvic muscles. Your next battle damage might be reduced!")
    } else {
        format!("❌ <b>Clench failed!</b> Your pelvic muscles weren't strong enough this time.")
    }
}

fn get_random_tip(lang_code: &LanguageCode) -> String {
    let mut rng = thread_rng();
    let tip_index = rng.gen_range(1..=10);
    let tip_key = format!("commands.tip.tip{}", tip_index);
    
    let tip = t!(&tip_key, locale = lang_code);
    format!("💡 <b>Anti-Hemorrhoid Tip:</b>\n\n{}", tip)
}
