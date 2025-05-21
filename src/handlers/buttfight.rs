use anyhow::{anyhow, Context};
use futures::join;
use rand::Rng;
use rand::rngs::OsRng;
use rust_i18n::t;
use teloxide::Bot;
use teloxide::macros::BotCommands;
use teloxide::payloads::AnswerInlineQuerySetters;
use teloxide::requests::Requester;
use teloxide::types::{CallbackQuery, ChatId, ChosenInlineResult, InlineKeyboardButton, InlineKeyboardMarkup, InlineQuery, InlineQueryResult, InlineQueryResultArticle, InputMessageContent, InputMessageContentText, Message, ParseMode, ReplyMarkup, User, UserId};
use crate::handlers::{CallbackResult, HandlerResult, reply_html, send_error_callback_answer, utils};
use crate::{metrics, reply_html, repo};
use crate::config::{AppConfig, BattlesFeatureToggles};
use crate::domain::{LanguageCode, Username};
use crate::handlers::utils::callbacks;
use crate::handlers::utils::callbacks::{CallbackDataWithPrefix, InvalidCallbackDataBuilder, NewLayoutValue};
use crate::handlers::utils::locks::LockCallbackServiceFacade;
use crate::repo::{BattleStats, ChatIdPartiality, TreatmentResult, Repositories, WinRateAware};

// let's calculate time offsets from 22.06.2024
const TIMESTAMP_MILLIS_SINCE_2024: i64 = 1719014400000;

#[derive(BotCommands, Clone, Copy)]
#[command(rename_rule = "lowercase")]
pub enum BattleCommands {
    #[command(description = "buttfight")]
    Penetrate(u16),
    Buttfight(u16),
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum BattleCommandsNoArgs {
    Penetrate,
    Buttfight,
}

impl BattleCommands {
    fn bet(&self) -> u16 {
        match *self {
            Self::Penetrate(bet) => bet,
            Self::Buttfight(bet) => bet,
        }
    }
}

#[derive(derive_more::Display)]
#[display("{initiator}:{bet}:{timestamp}")]
pub(crate) struct BattleCallbackData {
    initiator: UserId,
    bet: u16,

    // used to prevent repeated clicks on the same button
    timestamp: NewLayoutValue<i64>
}

impl BattleCallbackData {
    fn new(initiator: UserId, bet: u16) -> Self {
        Self {
            initiator, bet,
            timestamp: new_short_timestamp()
        }
    }
}

impl CallbackDataWithPrefix for BattleCallbackData {
    fn prefix() -> &'static str {
        "btf" // buttfight
    }
}

impl TryFrom<String> for BattleCallbackData {
    type Error = callbacks::InvalidCallbackData;

    fn try_from(data: String) -> Result<Self, Self::Error> {
        let err = InvalidCallbackDataBuilder(&data);
        let mut parts = data.split(':');
        let initiator = callbacks::parse_part(&mut parts, &err, "uid").map(UserId)?;
        let bet: u16 = callbacks::parse_part(&mut parts, &err, "bet")?;
        let timestamp = callbacks::parse_optional_part(&mut parts, &err)?;
        Ok(Self { initiator, bet, timestamp })
    }
}

pub async fn cmd_handler(bot: Bot, msg: Message, cmd: BattleCommands,
                         repos: Repositories, config: AppConfig) -> HandlerResult {
    metrics::CMD_PVP_COUNTER.chat.inc();

    let user = msg.from.as_ref().ok_or(anyhow!("no FROM field in the Penetrate command handler"))?.into();
    let lang_code = LanguageCode::from_maybe_user(msg.from.as_ref());
    let params = BattleParams {
        repos,
        features: config.features.pvp,
        chat_id: msg.chat.id.into(),
        lang_code,
    };
    let (text, keyboard) = buttfight_impl_start(params, user, cmd.bet()).await?;

    let mut answer = reply_html(bot, &msg, text);
    answer.reply_markup = keyboard.map(ReplyMarkup::InlineKeyboard);
    answer.await?;
    Ok(())
}

pub async fn cmd_handler_no_args(bot: Bot, msg: Message) -> HandlerResult {
    metrics::CMD_PVP_COUNTER.chat.inc();

    let lang_code = LanguageCode::from_maybe_user(msg.from.as_ref());
    reply_html!(bot, msg, t!("commands.penetrate.errors.no_args", locale = &lang_code));
    Ok(())
}

pub fn inline_filter(query: InlineQuery) -> bool {
    let maybe_bet: Result<u32, _> = query.query.parse();
    maybe_bet.is_ok()
}

pub fn chosen_inline_result_filter(result: ChosenInlineResult) -> bool {
    let maybe_bet: Result<u32, _> = result.query.parse();
    maybe_bet.is_ok()
}

pub async fn inline_handler(bot: Bot, query: InlineQuery) -> HandlerResult {
    metrics::INLINE_COUNTER.invoked();

    let bet: u16 = query.query.parse()?;
    let lang_code = LanguageCode::from_user(&query.from);
    let name = utils::get_full_name(&query.from);
    let res = build_inline_keyboard_article_result(query.from.id, &lang_code, &name, bet);

    let mut answer = bot.answer_inline_query(&query.id, vec![res.clone()])
        .is_personal(true);
    if cfg!(debug_assertions) {
        answer.cache_time.replace(1);
    }
    answer.await.context(format!("couldn't answer a callback query {query:?} with {res:?}"))?;
    Ok(())
}

pub(super) fn build_inline_keyboard_article_result(uid: UserId, lang_code: &LanguageCode, name: &Username, bet: u16) -> InlineQueryResult {
    log::debug!("Starting a buttfight for {uid} (bet = {bet})...");

    let title = t!("inline.results.titles.penetrate", locale = lang_code, bet = bet);
    let text = t!("commands.penetrate.results.start", locale = lang_code, name = name.escaped(), bet = bet);
    let content = InputMessageContent::Text(InputMessageContentText::new(text).parse_mode(ParseMode::Html));
    let btn_label = t!("commands.penetrate.button", locale = lang_code);
    let btn_data = BattleCallbackData::new(uid, bet).to_data_string();
    InlineQueryResultArticle::new("penetrate", title, content)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(btn_label, btn_data)
        ]]))
        .into()
}

pub async fn inline_chosen_handler() -> HandlerResult {
    metrics::INLINE_COUNTER.finished();
    Ok(())
}

#[inline]
pub fn callback_filter(query: CallbackQuery) -> bool {
    BattleCallbackData::check_prefix(query)
}

pub async fn callback_handler(bot: Bot, query: CallbackQuery, repos: Repositories, config: AppConfig,
                              mut battle_locker: LockCallbackServiceFacade) -> HandlerResult {
    let chat_id: ChatIdPartiality = query.message.as_ref()
        .map(|msg| msg.chat().id)
        .or_else(|| config.features.chats_merging
            .then_some(query.inline_message_id.as_ref())
            .flatten()
            .and_then(|msg_id| utils::resolve_inline_message_id(msg_id)
                .inspect_err(|e| log::error!("couldn't resolve inline_message_id: {e}"))
                .ok()
            )
            .map(|info| ChatId(info.chat_id))
        )
        .map(ChatIdPartiality::from)
        .unwrap_or(ChatIdPartiality::from(query.chat_instance.clone()));

    let callback_data = BattleCallbackData::parse(&query)?;
    if callback_data.initiator == query.from.id {
        return send_error_callback_answer(bot, query, "commands.penetrate.errors.same_person").await;
    }
    let _battle_guard = match battle_locker.try_lock(&callback_data) {
        Some(lock) => lock,
        None => return send_error_callback_answer(bot, query, "commands.penetrate.errors.battle_already_in_progress").await
    };

    let params = BattleParams {
        repos,
        features: config.features.pvp,
        lang_code: LanguageCode::from_user(&query.from),
        chat_id: chat_id.clone(),
    };
    let attack_result = buttfight_impl_attack(params, callback_data.initiator, query.from.clone().into(), callback_data.bet).await?;
    attack_result.apply(bot, query).await?;

    metrics::CMD_PVP_COUNTER.inline.inc();
    Ok(())
}

pub(crate) struct BattleParams {
    repos: Repositories,
    features: BattlesFeatureToggles,
    chat_id: ChatIdPartiality,
    lang_code: LanguageCode,
}

#[derive(Clone)]
pub(crate) struct UserInfo {
    uid: UserId,
    name: Username,
}

impl From<&User> for UserInfo {
    fn from(value: &User) -> Self {
        Self {
            uid: value.id,
            name: utils::get_full_name(value)
        }
    }
}

impl From<User> for UserInfo {
    fn from(value: User) -> Self {
        (&value).into()
    }
}

impl From<repo::User> for UserInfo {
    fn from(value: repo::User) -> Self {
        Self {
            uid: UserId(value.uid as u64),
            name: value.name
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<UserId> for UserInfo {
    fn into(self) -> UserId {
        self.uid
    }
}

pub(crate) async fn buttfight_impl_start(p: BattleParams, initiator: UserInfo, bet: u16) -> anyhow::Result<(String, Option<InlineKeyboardMarkup>)> {
    let enough = p.repos.hemoroids.check_hemoroid(&p.chat_id.kind(), initiator.uid, bet).await?;
    log::debug!("Starting a buttfight for {} in the chat with id = {} (bet = {bet}, enough = {enough})...", initiator.uid, p.chat_id);

    let data = if enough {
        let text = t!("commands.penetrate.results.start", locale = &p.lang_code, name = initiator.name.escaped(), bet = bet).to_string();
        let btn_label = t!("commands.penetrate.button", locale = &p.lang_code);
        let btn_data = BattleCallbackData::new(initiator.uid, bet).to_data_string();
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(btn_label, btn_data)
        ]]);
        (text, Some(keyboard))
    } else {
        (t!("commands.penetrate.errors.not_enough.initiator", locale = &p.lang_code).to_string(), None)
    };
    Ok(data)
}

async fn buttfight_impl_attack(p: BattleParams, initiator: UserId, acceptor: UserInfo, bet: u16) -> anyhow::Result<CallbackResult> {
    let chat_id_kind = p.chat_id.kind();
    let (enough_initiator, enough_acceptor) = join!(
       p.repos.hemoroids.check_hemoroid(&chat_id_kind, initiator, bet),
       p.repos.hemoroids.check_hemoroid(&chat_id_kind, acceptor.uid, if p.features.check_acceptor_length { bet } else { 0 }),
    );
    let (enough_initiator, enough_acceptor) = (enough_initiator?, enough_acceptor?);

    log::debug!("Executing the battle: chat_id = {}, initiator = {initiator} (enough = {enough_initiator}), acceptor = {} (enough = {enough_acceptor}), bet = {bet}...",
        p.chat_id, acceptor.uid);

    let result = if enough_initiator && enough_acceptor {
        // Randomly assign top and bottom roles
        let mut rng = rand::thread_rng();
        let initiator_is_top = rng.gen_bool(0.5);
        
        // Get user info
        let initiator_info = get_user_info(&p.repos.users, initiator, &acceptor).await?;
        let acceptor_info = get_user_info(&p.repos.users, acceptor.uid, &acceptor).await?;
        
        let (top_id, top_name, bottom_id, bottom_name) = if initiator_is_top {
            (initiator, initiator_info.name.escaped(), acceptor.uid, acceptor_info.name.escaped())
        } else {
            (acceptor.uid, acceptor_info.name.escaped(), initiator, initiator_info.name.escaped())
        };
        
        // Calculate damage
        let top_damage = calculate_top_damage(&mut rng);
        let bottom_damage = calculate_bottom_damage(&mut rng);
        
        // Apply damage to the users
        let (top_result, bottom_result) = p.repos.hemoroids.penetrate(&p.chat_id, top_id, bottom_id, top_damage, bottom_damage).await?;
        
        // Determine who received less damage (the winner)
        let (winner_id, winner_name, winner_damage, winner_level, loser_id, loser_name, loser_damage, loser_level) = 
            if top_damage.abs() < bottom_damage.abs() {
                (top_id, top_name, top_damage, top_result.new_protrusion_level, 
                 bottom_id, bottom_name, bottom_damage, bottom_result.new_protrusion_level)
            } else {
                (bottom_id, bottom_name, bottom_damage, bottom_result.new_protrusion_level, 
                 top_id, top_name, top_damage, top_result.new_protrusion_level)
            };
        
        let battle_stats = p.repos.pvp_stats.send_battle_result(&p.chat_id.kind(), winner_id, loser_id, bet).await
            .inspect_err(|e| log::error!("couldn't send users' battle statistics for winner ({}) and loser ({}): {}", winner_id, loser_id, e))
            .ok()
            .filter(|_| p.features.show_stats)
            .map(|BattleStats { winner: winner_stats, loser: loser_stats }| {
                let mut stats_str = t!("commands.penetrate.results.stats.text", locale = &p.lang_code,
                    winner_win_rate = winner_stats.win_rate_formatted(), loser_win_rate = loser_stats.win_rate_formatted(),
                    winner_win_streak = winner_stats.win_streak_current, winner_win_streak_max = winner_stats.win_streak_max,
                ).to_string();
                if loser_stats.prev_win_streak > 1 {
                    stats_str.push('\n');
                    stats_str.push_str(&t!("commands.penetrate.results.stats.lost_win_streak", locale = &p.lang_code,
                        lost_win_streak = loser_stats.prev_win_streak));
                }
                stats_str
            })
            .map(|s| format!("\n\n{s}"))
            .unwrap_or_default();
        
        // Generate battle description
        let outcome = generate_battle_outcome(&p.lang_code, top_id, top_name, top_damage, 
                                            bottom_id, bottom_name, bottom_damage,
                                            winner_id, winner_name, loser_name,
                                            bet);
        
        // Create position information if available
        let positions = if let (Some(winner_pos), Some(loser_pos)) = (top_result.pos_in_top, bottom_result.pos_in_top) {
            let winner_pos = t!("commands.penetrate.results.position", locale = &p.lang_code, name = winner_name, pos = winner_pos);
            let loser_pos = t!("commands.penetrate.results.position", locale = &p.lang_code, name = loser_name, pos = loser_pos);
            format!("\n\n{winner_pos}\n{loser_pos}")
        } else {
            String::new()
        };
        
        CallbackResult::EditMessage(format!("{outcome}{positions}{battle_stats}"), None)
    } else if enough_acceptor {
        let text = t!("commands.penetrate.errors.not_enough.initiator", locale = &p.lang_code).to_string();
        CallbackResult::EditMessage(text, None)
    } else {
        let text = t!("commands.penetrate.errors.not_enough.acceptor", locale = &p.lang_code).to_string();
        CallbackResult::ShowError(text)
    };
    Ok(result)
}

fn calculate_top_damage(rng: &mut impl Rng) -> i32 {
    // Top typically receives minor damage (0.1-0.4cm)
    // Positive = bad (swelling), negative = good (shrinking)
    let critical_event = rng.gen_bool(0.1); // 10% chance of critical event
    
    if critical_event {
        if rng.gen_bool(0.4) { // 4% overall chance of being lucky
            -5 // Lucky! Significant healing
        } else { // 6% overall chance of complications
            rng.gen_range(5..10) // Complications! Major swelling
        }
    } else {
        rng.gen_range(1..5) // Normal minor damage
    }
}

fn calculate_bottom_damage(rng: &mut impl Rng) -> i32 {
    // Bottom typically receives major damage (0.4-1.8cm)
    // Positive = bad (swelling), negative = good (shrinking)
    let critical_event = rng.gen_bool(0.15); // 15% chance of critical event
    
    if critical_event {
        if rng.gen_bool(0.3) { // 4.5% overall chance of being lucky
            -10 // Lucky! Significant healing
        } else { // 10.5% overall chance of severe trauma
            rng.gen_range(18..25) // Trauma! Severe swelling
        }
    } else {
        rng.gen_range(4..19) // Normal major damage
    }
}

fn generate_battle_outcome(lang_code: &LanguageCode, 
                           top_id: UserId, top_name: String, top_damage: i32,
                           bottom_id: UserId, bottom_name: String, bottom_damage: i32,
                           winner_id: UserId, winner_name: String, loser_name: String,
                           bet: u16) -> String {
    // Create battle description
    let top_outcome = if top_damage > 0 {
        t!("commands.penetrate.results.top_swelled", 
          locale = lang_code,
          name = top_name,
          damage = top_damage.abs())
    } else {
        t!("commands.penetrate.results.top_improved", 
          locale = lang_code,
          name = top_name,
          improvement = top_damage.abs())
    };
    
    let bottom_outcome = if bottom_damage > 0 {
        t!("commands.penetrate.results.bottom_swelled", 
          locale = lang_code,
          name = bottom_name,
          damage = bottom_damage.abs())
    } else {
        t!("commands.penetrate.results.bottom_improved", 
          locale = lang_code,
          name = bottom_name,
          improvement = bottom_damage.abs())
    };
    
    let winner_msg = t!("commands.penetrate.results.winner", 
                       locale = lang_code,
                       name = winner_name,
                       bet = bet);
    
    format!(
        "{}\n\n{}\n\n{}",
        top_outcome,
        bottom_outcome,
        winner_msg
    )
}

async fn get_user_info(users: &repo::Users, user_uid: UserId, acceptor: &UserInfo) -> anyhow::Result<UserInfo> {
    let user = if user_uid == acceptor.uid {
        acceptor.clone()
    } else {
        users.get(user_uid).await?
            .ok_or(anyhow!("buttfight participant must be present in the database!"))?
            .into()
    };
    Ok(user)
}

pub fn new_short_timestamp() -> NewLayoutValue<i64> {
    NewLayoutValue::Some(chrono::Utc::now().timestamp_millis() - TIMESTAMP_MILLIS_SINCE_2024)
}
