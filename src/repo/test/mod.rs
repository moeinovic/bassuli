mod users;
mod hemoroids;
mod chats;
mod import;
mod promo;
mod loans;
mod pvpstats;
mod stats;
mod announcements;

use std::str::FromStr;
use reqwest::Url;
use sqlx::{Pool, Postgres};
use teloxide::types::{ChatId, UserId};
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use crate::config::DatabaseConfig;
use crate::repo;
use crate::repo::ChatIdKind;

const POSTGRES_USER: &str = "test";
const POSTGRES_PASSWORD: &str = "test_pw";
const POSTGRES_DB: &str = "test_db";
const POSTGRES_PORT: u16 = 5432;

pub const UID: i64 = 12345;
pub const CHAT_ID: i64 = 67890;
pub const NAME: &str = "test";

pub const USER_ID: UserId = UserId(UID as u64);
pub const CHAT_ID_KIND: ChatIdKind = ChatIdKind::ID(ChatId(CHAT_ID));

pub async fn start_postgres() -> (ContainerAsync<GenericImage>, Pool<Postgres>) {
    let postgres_container = GenericImage::new("postgres", "latest")
        .with_exposed_port(POSTGRES_PORT.tcp())
        .with_wait_for(WaitFor::message_on_stdout("PostgreSQL init process complete; ready for start up."))
        .with_wait_for(WaitFor::message_on_stdout("PostgreSQL init process complete; ready for start up."))
        .with_wait_for(WaitFor::millis(300))
        .with_env_var("POSTGRES_USER", POSTGRES_USER)
        .with_env_var("POSTGRES_PASSWORD", POSTGRES_PASSWORD)
        .with_env_var("POSTGRES_DB", POSTGRES_DB)
        .start()
        .await
        .expect("couldn't start Postgres database");

    let postgres_port = postgres_container.get_host_port_ipv4(POSTGRES_PORT)
        .await
        .expect("couldn't fetch port from PostgreSQL server");
    let db_url = Url::from_str(&format!("postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@localhost:{postgres_port}/{POSTGRES_DB}"))
        .expect("invalid database URL");
    let conf = DatabaseConfig{
        url: db_url,
        max_connections: 10,
    };
    let pool = repo::establish_database_connection(&conf)
        .await.expect("couldn't establish a database connection");
    (postgres_container, pool)
}

#[inline]
pub fn get_chat_id_and_hemoroids(db: &Pool<Postgres>) -> (ChatIdKind, repo::Hemoroids) {
    let hemoroids_repo = repo::Hemoroids::new(db.clone(), Default::default());
    let chat_id = ChatIdKind::ID(ChatId(CHAT_ID));
    (chat_id, hemoroids_repo)
}
