use anyhow::{anyhow, Context};
use futures::TryFutureExt;
use rand::Rng;
use sqlx::{Executor, Pool, Postgres, Transaction};
use teloxide::types::UserId;
use crate::config::FeatureToggles;
use super::{ChatIdKind, ChatIdPartiality, Chats, UID};

#[derive(sqlx::FromRow, Debug)]
pub struct Hemoroid {
    pub protrusion_level: i32,
    pub owner_uid: UID,
    pub owner_name: String,
    pub treated_at: chrono::DateTime<chrono::Utc>,
    pub position: Option<i64>,
}

pub struct TreatmentResult {
    pub new_protrusion_level: i32,
    pub pos_in_top: Option<u64>,
}

#[derive(Clone)]
pub struct Hemoroids {
    pool: Pool<Postgres>,
    chats: Chats,
    features: FeatureToggles,
}

impl Hemoroids {
    pub fn new(pool: Pool<Postgres>, features: FeatureToggles) -> Self {
        Self {
            chats: Chats::new(pool.clone(), features),
            pool,
            features,
        }
    }

    // Initialize new user with random hemoroid protrusion level between 2.5 and 5.0 cm
    pub async fn initialize_new_user(&self, uid: UserId, chat_id: &ChatIdPartiality) -> anyhow::Result<TreatmentResult> {
        let uid = uid.0 as i64;
        let internal_chat_id = self.chats.upsert_chat(chat_id).await?;
        let mut rng = rand::thread_rng();
        let initial_level = (rng.gen_range(25..51) as f32 / 10.0) as i32;

        let protrusion_level = sqlx::query_scalar!(
            "INSERT INTO Hemoroids(uid, chat_id, protrusion_level, updated_at) VALUES ($1, $2, $3, current_timestamp)
                ON CONFLICT (uid, chat_id) DO UPDATE SET protrusion_level = $3, updated_at = current_timestamp
                RETURNING protrusion_level",
                uid, internal_chat_id, initial_level)
            .fetch_one(&self.pool)
            .await
            .context(format!("couldn't initialize hemorrhoid of {uid} in {chat_id} with level of {initial_level}"))?;
        
        let pos_in_top = self.get_position_in_top(internal_chat_id, uid).await?;
        
        Ok(TreatmentResult { new_protrusion_level: protrusion_level, pos_in_top })
    }

    pub async fn create_or_shrink(&self, uid: UserId, chat_id: &ChatIdPartiality, change: i32) -> anyhow::Result<TreatmentResult> {
        let uid = uid.0 as i64;
        let internal_chat_id = self.chats.upsert_chat(chat_id).await?;
        
        // Determine if user exists
        let user_exists = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM Hemoroids WHERE uid = $1 AND chat_id = $2",
            uid, internal_chat_id
        )
        .fetch_one(&self.pool)
        .await?
        .unwrap_or(0) > 0;
        
        // If new user, initialize with random value, otherwise apply change
        if !user_exists {
            return self.initialize_new_user(UserId(uid as u64), chat_id).await;
        }
        
        let new_protrusion_level = sqlx::query_scalar!(
            "INSERT INTO Hemoroids(uid, chat_id, protrusion_level, updated_at) VALUES ($1, $2, $3, current_timestamp)
                ON CONFLICT (uid, chat_id) DO UPDATE SET protrusion_level = (Hemoroids.protrusion_level + $3), updated_at = current_timestamp
                RETURNING protrusion_level",
                uid, internal_chat_id, change)
            .fetch_one(&self.pool)
            .await
            .context(format!("couldn't update the hemorrhoid of {uid} in {chat_id} with change of {change}"))?;
        
        let pos_in_top = self.get_position_in_top(internal_chat_id, uid).await?;
        Ok(TreatmentResult { new_protrusion_level, pos_in_top })
    }

    pub async fn fetch_protrusion_level(&self, uid: UserId, chat_id: &ChatIdKind) -> anyhow::Result<i32> {
        sqlx::query_scalar!("SELECT h.protrusion_level FROM Hemoroids h \
                JOIN Chats c ON h.chat_id = c.id \
                WHERE uid = $1 AND \
                    c.chat_id = $2::bigint OR c.chat_instance = $2::text",
                uid.0 as i64, chat_id.value() as String)
            .fetch_optional(&self.pool)
            .await
            .map(|opt| opt.unwrap_or(0))
            .context(format!("couldn't fetch protrusion_level for {chat_id} and {uid}"))
    }

    pub async fn fetch_hemoroid(&self, uid: UserId, chat_id: &ChatIdKind) -> anyhow::Result<Option<Hemoroid>> {
        sqlx::query_as!(Hemoroid,
            r#"SELECT protrusion_level, uid as owner_uid, name as owner_name, updated_at as treated_at, position FROM (
                 SELECT uid, name, h.protrusion_level as protrusion_level, updated_at, ROW_NUMBER() OVER (ORDER BY protrusion_level ASC, updated_at DESC, name) AS position
                   FROM Hemoroids h
                   JOIN users using (uid)
                   JOIN Chats c ON h.chat_id = c.id
                   WHERE c.chat_id = $2::bigint OR c.chat_instance = $2::text
               ) AS _
               WHERE uid = $1"#,
                uid.0 as i64, chat_id.value() as String)
            .fetch_optional(&self.pool)
            .await
            .context(format!("couldn't fetch hemorrhoid for {chat_id} and {uid}"))
    }

    pub async fn get_top(&self, chat_id: &ChatIdKind, offset: u32, limit: u16) -> anyhow::Result<Vec<Hemoroid>> {
        sqlx::query_as!(Hemoroid,
            r#"SELECT protrusion_level, uid as owner_uid, name as owner_name, updated_at as treated_at,
                    ROW_NUMBER() OVER (ORDER BY protrusion_level ASC, updated_at DESC, name) AS position
                FROM Hemoroids h
                JOIN users using (uid)
                JOIN chats c ON c.id = h.chat_id
                WHERE c.chat_id = $1::bigint OR c.chat_instance = $1::text
                OFFSET $2 LIMIT $3"#,
                chat_id.value() as String, offset as i64, limit as i32)
            .fetch_all(&self.pool)
            .await
            .context(format!("couldn't get the top of {chat_id} with offset = {offset} and limit = {limit}"))
    }

    pub async fn get_worst(&self, chat_id: &ChatIdKind, offset: u32, limit: u16) -> anyhow::Result<Vec<Hemoroid>> {
        sqlx::query_as!(Hemoroid,
            r#"SELECT protrusion_level, uid as owner_uid, name as owner_name, updated_at as treated_at,
                    ROW_NUMBER() OVER (ORDER BY protrusion_level DESC, updated_at ASC, name) AS position
                FROM Hemoroids h
                JOIN users using (uid)
                JOIN chats c ON c.id = h.chat_id
                WHERE c.chat_id = $1::bigint OR c.chat_instance = $1::text
                OFFSET $2 LIMIT $3"#,
                chat_id.value() as String, offset as i64, limit as i32)
            .fetch_all(&self.pool)
            .await
            .context(format!("couldn't get the worst of {chat_id} with offset = {offset} and limit = {limit}"))
    }

    pub async fn set_hod_winner(&self, chat_id: &ChatIdPartiality, user_id: UserId, improvement: u16) -> anyhow::Result<Option<TreatmentResult>> {
        let internal_chat_id = self.chats.upsert_chat(chat_id).await?;

        let mut tx = self.pool.begin().await?;
        let uid = user_id.0 as i64;
        // Note: improvement is negative since we want to reduce protrusion level
        let new_protrusion_level = match Self::shrink_no_attempts_check_internal(&mut *tx, internal_chat_id, uid, -(improvement as i32)).await? {
            Some(level) => level,
            None => return Ok(None)
        };
        Self::insert_to_hod_table(&mut tx, internal_chat_id, uid).await?;
        tx.commit().await?;

        let pos_in_top = self.get_position_in_top(internal_chat_id, uid).await?;
        Ok(Some(TreatmentResult { new_protrusion_level, pos_in_top }))
    }

    pub async fn check_hemoroid(&self, chat_id: &ChatIdKind, user_id: UserId, level: u16) -> anyhow::Result<bool> {
        sqlx::query_scalar!(r#"SELECT protrusion_level <= $3 AS "enough!" FROM Hemoroids h
                JOIN Chats c ON h.chat_id = c.id
                WHERE (c.chat_id = $1::bigint OR c.chat_instance = $1::text)
                    AND uid = $2"#,
                chat_id.value() as String, user_id.0 as i64, level as i32)
            .fetch_optional(&self.pool)
            .map_ok(|opt| opt.unwrap_or(false))
            .await
            .context(format!("couldn't check the hemorrhoid {chat_id}, {user_id} to have at most {level} cm"))
    }

    pub async fn penetrate(&self, chat_id: &ChatIdPartiality, top: UserId, bottom: UserId, top_damage: i32, bottom_damage: i32) -> anyhow::Result<(TreatmentResult, TreatmentResult)> {
        let internal_chat_id = self.chats.upsert_chat(chat_id).await?;

        let mut tx = self.pool.begin().await?;
        let level_top = Self::damage_for_one_user(&mut tx, internal_chat_id, top.0, top_damage).await?;
        let level_bottom = Self::damage_for_one_user(&mut tx, internal_chat_id, bottom.0, bottom_damage).await?;
        tx.commit().await?;

        let pos_top = self.get_position_in_top(internal_chat_id, top.0 as i64).await?;
        let pos_bottom = self.get_position_in_top(internal_chat_id, bottom.0 as i64).await?;
        
        let top_result = TreatmentResult {
            new_protrusion_level: level_top,
            pos_in_top: pos_top,
        };
        let bottom_result = TreatmentResult {
            new_protrusion_level: level_bottom,
            pos_in_top: pos_bottom,
        };
        Ok((top_result, bottom_result))
    }

    async fn damage_for_one_user(tx: &mut Transaction<'_, Postgres>, chat_id_internal: i64, user_id: u64, damage: i32) -> anyhow::Result<i32> {
        sqlx::query_scalar!("UPDATE Hemoroids SET protrusion_level = (protrusion_level + $3), bonus_attempts = (bonus_attempts + 1) WHERE chat_id = $1 AND uid = $2 RETURNING protrusion_level",
                    chat_id_internal, user_id as i64, damage)
            .fetch_one(&mut **tx)
            .await
            .context(format!("couldn't update the protrusion_level by {damage} for {chat_id_internal}, {user_id}"))
    }

    async fn get_position_in_top(&self, chat_id_internal: i64, uid: i64) -> anyhow::Result<Option<u64>> {
        if !self.features.top_unlimited {
            return Ok(None)
        }
        sqlx::query_scalar!(
                r#"SELECT position AS "position!" FROM (
                    SELECT uid, ROW_NUMBER() OVER (ORDER BY protrusion_level ASC, updated_at DESC, name) AS position
                    FROM Hemoroids
                    JOIN users using (uid)
                    WHERE chat_id = $1
                ) AS _
                WHERE uid = $2"#,
                chat_id_internal, uid)
            .fetch_one(&self.pool)
            .await
            .map(|pos| Some(pos as u64))
            .context(format!("couldn't get the top position for {chat_id_internal} and {uid}"))
    }
    
    pub async fn shrink_no_attempts_check(&self, chat_id: &ChatIdKind, user_id: UserId, change: i32) -> anyhow::Result<TreatmentResult> {
        let chat_internal_id = self.chats.get_internal_id(chat_id).await?;
        let uid = user_id.0 as i64;
    
        let new_protrusion_level = Self::shrink_no_attempts_check_internal(&self.pool, chat_internal_id, uid, change).await?
            .ok_or(anyhow!("couldn't find a hemorrhoid of ({chat_id}, {uid}) for some reason"))?;
        let pos_in_top = self.get_position_in_top(chat_internal_id, uid).await?;
        
        Ok(TreatmentResult { new_protrusion_level, pos_in_top })
    }

    pub(super) async fn shrink_no_attempts_check_internal<'c, E>(executor: E, chat_id_internal: i64, user_id: i64, change: i32) -> anyhow::Result<Option<i32>>
    where E: Executor<'c, Database = Postgres>,
    {
        sqlx::query_scalar!(
            "UPDATE Hemoroids SET bonus_attempts = (bonus_attempts + 1), protrusion_level = (protrusion_level + $3)
                WHERE chat_id = $1 AND uid = $2
                RETURNING protrusion_level",
                chat_id_internal, user_id, change)
            .fetch_optional(executor)
            .await
            .context(format!("couldn't update the hemorrhoid protrusion without attempts check for {chat_id_internal} and {user_id} by {change}"))
    }

    async fn insert_to_hod_table(tx: &mut Transaction<'_, Postgres>, chat_id_internal: i64, user_id: i64) -> anyhow::Result<()> {
        sqlx::query!("INSERT INTO Hemoroid_of_Day (chat_id, lowest_uid) VALUES ($1, $2)",
                chat_id_internal, user_id)
            .execute(&mut **tx)
            .await
            .context(format!("couldn't insert to HOD table for {chat_id_internal} and {user_id}"))?;
        Ok(())
    }
}
