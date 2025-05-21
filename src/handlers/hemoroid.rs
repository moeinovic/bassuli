use std::future::IntoFuture;

use anyhow::{anyhow, Context};
use chrono::{Datelike, Utc};
use futures::future::join;
use futures::TryFutureExt;
use rust_i18n::t;
use teloxide::Bot;
use teloxide::macros::BotCommands;
use teloxide::requests::Requester;
use teloxide::types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message, ParseMode, ReplyMarkup, User, UserId};

use page::{InvalidPage, Page};

use crate::{config, metrics, repo};
use crate::domain::{LanguageCode, Username};
use crate::handlers::{HandlerResult, reply_html, utils};
use crate::handlers::utils::{callbacks, Incrementor, page};
use crate::repo::{ChatIdPartiality, UID};

const TOMORROW_SQL_CODE: &str = "GD0E1";
const CALLBACK_PREFIX_TOP_PAGE: &str = "top:page:";

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum HemoroidCommands {
    #[command(description = "grow")]
    Grow,
    #[command(description = "top")]
    Top,
}

pub async fn hemoroid_cmd_handler(bot: Bot, msg: Message, cmd: HemoroidCommands,
                              repos: repo::Repositories, incr: Incrementor,
                              config: config::AppConfig) -> HandlerResult {
    let from = msg.from.as_ref().ok_or(anyhow!("unexpected absence of a FROM field"))?;
    let chat_id = msg.chat.id.into();
    let from_refs = FromRefs(from, &chat_id);
    match cmd {
        HemoroidCommands::Grow => {
            metrics::CMD_GROW_COUNTER.chat.inc();
            let answer = grow_impl(&repos, incr, from_refs).await?;
            reply_html(bot, &msg, answer)
        },
        HemoroidCommands::Top => {
            metrics::CMD_TOP_COUNTER.chat.inc();
            let top = top_impl(&repos, &config, from_refs, Page::first()).await?;
            let mut request = reply_html(bot, &msg, top.lines);
            if top.has_more_pages && config.features.top_unlimited {
                let keyboard = ReplyMarkup::InlineKeyboard(build_pagination_keyboard(Page::first(), top.has_more_pages));
                request.reply_markup.replace(keyboard);
            }
            request
        }
    }.await.context(format!("failed for {msg:?}"))?;
    Ok(())
}

pub struct FromRefs<'a>(pub &'a User, pub &'a ChatIdPartiality);

pub(crate) async fn grow_impl(repos: &repo::Repositories, incr: Incrementor, from_refs: FromRefs<'_>) -> anyhow::Result<String> {
    let (from, chat_id) = (from_refs.0, from_refs.1);
    let name = utils::get_full_name(from);
    let user = repos.users.create_or_update(from.id, &name).await?;
    let days_since_registration = (Utc::now() - user.created_at).num_days() as u32;
    let increment = incr.growth_increment(from.id, chat_id.kind(), days_since_registration).await;
    let grow_result = repos.hemoroids.create_or_grow(from.id, chat_id, increment.total).await;
    let lang_code = LanguageCode::from_user(from);

    let main_part = match grow_result {
        Ok(repo::GrowthResult { new_length, pos_in_top }) => {
            let event_key_for_i18n = if increment.total.is_negative() { "shrunk" } else { "swelled" };
            let event_template = format!("commands.shrink.direction.{}", event_key_for_i18n);
            let event = t!(&event_template, locale = &lang_code);
            let answer = t!("commands.shrink.result", locale = &lang_code,
                event = event, incr = increment.total.abs(), length = new_length);
            let perks_part = increment.perks_part_of_answer(&lang_code);
            if let Some(pos) = pos_in_top {
                let position = t!("commands.shrink.position", locale = &lang_code, pos = pos);
                format!("{answer}\n{position}{perks_part}")
            } else {
                format!("{answer}{perks_part}")
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
        .map(|(i, d)| {
            let escaped_name = Username::new(d.owner_name).escaped();
            let name = if from.id == <UID as Into<UserId>>::into(d.owner_uid) {
                format!("<u>{escaped_name}</u>")
            } else {
                escaped_name
            };
            let can_grow = Utc::now().num_days_from_ce() > d.grown_at.num_days_from_ce();
            let pos = d.position.unwrap_or((i+1) as i64);
            let mut line = t!("commands.top.line", locale = &lang_code,
                n = pos, name = name, length = d.length).to_string();
            if can_grow {
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

pub fn page_callback_filter(query: CallbackQuery) -> bool {
    query.data
        .filter(|d| d.starts_with(CALLBACK_PREFIX_TOP_PAGE))
        .is_some()
}

pub async fn page_callback_handler(bot: Bot, q: CallbackQuery,
                                   config: config::AppConfig, repos: repo::Repositories) -> HandlerResult {
    let edit_msg_req_params = callbacks::get_params_for_message_edit(&q)?;
    if !config.features.top_unlimited {
        return answer_callback_feature_disabled(bot, &q, edit_msg_req_params).await
    }

    let page = q.data.as_ref()
        .ok_or(InvalidPage::message("no data"))
        .and_then(|d| d.strip_prefix(CALLBACK_PREFIX_TOP_PAGE)
            .map(str::to_owned)
            .ok_or(InvalidPage::for_value(d, "invalid prefix")))
        .and_then(|r| r.parse()
            .map_err(|e| InvalidPage::for_value(&r, e)))
        .map(Page)
        .map_err(|e| anyhow!(e))?;
    let chat_id_kind = edit_msg_req_params.clone().into();
    let chat_id_partiality = ChatIdPartiality::Specific(chat_id_kind);
    let from_refs = FromRefs(&q.from, &chat_id_partiality);
    let top = top_impl(&repos, &config, from_refs, page).await?;

    let keyboard = build_pagination_keyboard(page, top.has_more_pages);
    let (answer_callback_query_result, edit_message_result) = match &edit_msg_req_params {
        callbacks::EditMessageReqParamsKind::Chat(chat_id, message_id) => {
            let mut edit_message_text_req = bot.edit_message_text(*chat_id, *message_id, top.lines);
            edit_message_text_req.parse_mode.replace(ParseMode::Html);
            edit_message_text_req.reply_markup.replace(keyboard);
            join(
                bot.answer_callback_query(&q.id).into_future(),
                edit_message_text_req.into_future().map_ok(|_| ())
            ).await
        },
        callbacks::EditMessageReqParamsKind::Inline { inline_message_id, .. } => {
            let mut edit_message_text_inline_req = bot.edit_message_text_inline(inline_message_id, top.lines);
            edit_message_text_inline_req.parse_mode.replace(ParseMode::Html);
            edit_message_text_inline_req.reply_markup.replace(keyboard);
            join(
                bot.answer_callback_query(&q.id).into_future(),
                edit_message_text_inline_req.into_future().map_ok(|_| ())
            ).await
        }
    };
    answer_callback_query_result.context(format!("failed to answer a callback query {q:?}"))?;
    edit_message_result.context(format!("failed to edit the message of {edit_msg_req_params:?}"))?;
    Ok(())
}

pub fn build_pagination_keyboard(page: Page, has_more_pages: bool) -> InlineKeyboardMarkup {
    let mut buttons = Vec::new();
    if page > 0 {
        buttons.push(InlineKeyboardButton::callback("⬅️", format!("{CALLBACK_PREFIX_TOP_PAGE}{}", page - 1)))
    }
    if has_more_pages {
        buttons.push(InlineKeyboardButton::callback("➡️", format!("{CALLBACK_PREFIX_TOP_PAGE}{}", page + 1)))
    }
    InlineKeyboardMarkup::new(vec![buttons])
}

async fn answer_callback_feature_disabled(bot: Bot, q: &CallbackQuery, edit_msg_req_params: callbacks::EditMessageReqParamsKind) -> HandlerResult {
    let lang_code = LanguageCode::from_user(&q.from);

    let mut answer = bot.answer_callback_query(&q.id);
    answer.show_alert.replace(true);
    answer.text.replace(t!("errors.feature_disabled", locale = &lang_code).to_string());
    answer.await?;

    match edit_msg_req_params {
        callbacks::EditMessageReqParamsKind::Chat(chat_id, message_id) =>
            bot.edit_message_reply_markup(chat_id, message_id)
                .await.map(|_| ())?,
        callbacks::EditMessageReqParamsKind::Inline { inline_message_id, .. } =>
            bot.edit_message_reply_markup_inline(inline_message_id)
                .await.map(|_| ())?
    };
    Ok(())
}
