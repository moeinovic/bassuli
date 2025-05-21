use num_traits::ToPrimitive;
use sqlx::{Pool, Postgres};
use teloxide::types::{ChatId, UserId};
use crate::config::FeatureToggles;
use crate::repo;
use crate::repo::{ChatIdKind, ChatIdPartiality};
use crate::repo::test::{CHAT_ID, get_chat_id_and_dicks, NAME, start_postgres, UID};

#[tokio::test]
async fn test_all() {
    let (_container, db) = start_postgres().await;
    let hemoroids_repo = repo::Hemoroids::new(db.clone(), Default::default());
    create_user(&db).await;

    let user_id = UserId(UID as u64);
    let chat_id = ChatIdKind::ID(ChatId(CHAT_ID));
    let chat_id_partiality = chat_id.clone().into();
    let d = hemoroids_repo.get_top(&chat_id, 0, 1)
        .await.expect("couldn't fetch the empty top");
    assert_eq!(d.len(), 0);

    let increment = 5;
    let growth = hemoroids_repo.create_or_grow(user_id, &chat_id_partiality, increment)
        .await.expect("couldn't grow a hemoroid");
    assert_eq!(growth.pos_in_top, Some(1));
    assert_eq!(growth.new_length, increment);
    check_top(&hemoroids_repo, &chat_id, increment).await;

    let growth = hemoroids_repo.set_hod_winner(&chat_id_partiality, user_id, increment as u16)
        .await
        .expect("couldn't elect a winner")
        .expect("the winner hasn't a hemoroid");
    assert_eq!(growth.pos_in_top, Some(1));
    let new_length = 2 * increment;
    assert_eq!(growth.new_length, new_length);
    check_top(&hemoroids_repo, &chat_id, new_length).await;
}

#[tokio::test]
async fn test_all_with_top_pagination_disabled() {
    let (_container, db) = start_postgres().await;
    let hemoroids_repo = {
        let features = FeatureToggles {
            top_unlimited: false,
            ..Default::default()
        };
        repo::Hemoroids::new(db.clone(), features)
    };
    create_user(&db).await;

    let user_id = UserId(UID as u64);
    let chat_id = ChatIdKind::ID(ChatId(CHAT_ID));
    let chat_id_partiality = chat_id.clone().into();
    let d = hemoroids_repo.get_top(&chat_id, 0, 1)
        .await.expect("couldn't fetch the empty top");
    assert_eq!(d.len(), 0);

    let increment = 5;
    let growth = hemoroids_repo.create_or_grow(user_id, &chat_id_partiality, increment)
        .await.expect("couldn't grow a hemoroid");
    assert_eq!(growth.pos_in_top, None);
    assert_eq!(growth.new_length, increment);
    check_top(&hemoroids_repo, &chat_id, increment).await;

    let growth = hemoroids_repo.set_hod_winner(&chat_id_partiality, user_id, increment as u16)
        .await
        .expect("couldn't elect a winner")
        .expect("the winner hasn't a hemoroid");
    assert_eq!(growth.pos_in_top, None);
    let new_length = 2 * increment;
    assert_eq!(growth.new_length, new_length);
    check_top(&hemoroids_repo, &chat_id, new_length).await;
}

#[tokio::test]
async fn test_top_page() {
    let (_container, db) = start_postgres().await;
    let hemoroids_repo = repo::Hemoroids::new(db.clone(), Default::default());
    let chat_id = ChatIdKind::ID(ChatId(CHAT_ID));
    let chat_id_partiality = chat_id.clone().into();
    let user2_name = format!("{NAME} 2");

    // create user and hemoroid #1
    create_user(&db).await;
    create_hemoroid(&db).await;
    // create user and hemoroid #2
    create_user_and_hemoroid_2(&db, &chat_id_partiality, &user2_name).await;

    let top_with_user2_only = hemoroids_repo.get_top(&chat_id, 0, 1)
        .await.expect("couldn't fetch the top");
    assert_eq!(top_with_user2_only.len(), 1);
    assert_eq!(top_with_user2_only[0].owner_name, user2_name);
    assert_eq!(top_with_user2_only[0].protrusion_level, 1);

    let top_with_user1_only = hemoroids_repo.get_top(&chat_id, 1, 1)
        .await.expect("couldn't fetch the top");
    assert_eq!(top_with_user1_only.len(), 1);
    assert_eq!(top_with_user1_only[0].owner_name, NAME);
    assert_eq!(top_with_user1_only[0].protrusion_level, 0);
}

#[tokio::test]
async fn test_pvp() {
    let (_container, db) = start_postgres().await;
    let hemoroids_repo = repo::Hemoroids::new(db.clone(), Default::default());
    let chat_id = ChatIdKind::ID(ChatId(CHAT_ID));
    let chat_id_part: &ChatIdPartiality = &chat_id.clone().into();
    let uid = UserId(UID as u64);
    {
        let enough = hemoroids_repo.check_hemoroid(&chat_id_part.kind(), uid, 1)
            .await.expect("couldn't check the hemoroid #1");
        assert!(!enough);
    }
    {
        create_user(&db).await;
        hemoroids_repo.create_or_grow(uid, chat_id_part, 1)
            .await
            .expect("couldn't create a hemoroid");

        let enough = hemoroids_repo.check_hemoroid(&chat_id_part.kind(), uid, 1)
            .await.expect("couldn't check the hemoroid #2");
        assert!(enough);
    }
    {
        let enough = hemoroids_repo.check_hemoroid(&chat_id_part.kind(), uid, 2)
            .await.expect("couldn't check the hemoroid #3");
        assert!(!enough);
    }
    {
        create_user_and_hemoroid_2(&db, chat_id_part, Default::default()).await;
        let uid2 = UserId((UID + 1) as u64);
        let (gr1, gr2) = hemoroids_repo.move_protrusion_level(chat_id_part, uid, uid2, 1)
            .await.expect("couldn't move the protrusion_level");

        assert_eq!(gr1.new_length, 0);
        assert_eq!(gr2.new_length, 2);
        assert_eq!(gr2.pos_in_top, Some(1));
        assert_eq!(gr1.pos_in_top, Some(2));
    }
}

pub async fn create_user(db: &Pool<Postgres>) {
    let users = repo::Users::new(db.clone());
    users.create_or_update(UserId(UID as u64), NAME)
        .await.expect("couldn't create a user");
}

pub async fn create_user_and_hemoroid_2(db: &Pool<Postgres>, chat_id: &ChatIdPartiality, name: &str) {
    create_another_user_and_hemoroid(db, chat_id, 2, name, 1).await;
}

pub async fn create_another_user_and_hemoroid(db: &Pool<Postgres>, chat_id: &ChatIdPartiality,
                                          n: u8, name: &str, increment: i32) {
    assert!(n > 1);
    let n = n.to_i64().expect("couldn't convert n to i64");
    
    let users = repo::Users::new(db.clone());
    let hemoroids_repo = repo::Hemoroids::new(db.clone(), Default::default());
    let uid2 = UserId((UID + n - 1) as u64);
    users.create_or_update(uid2, name)
        .await.unwrap_or_else(|_| panic!("couldn't create a user #{n}"));
    hemoroids_repo.create_or_grow(uid2, chat_id, increment)
        .await.unwrap_or_else(|_| panic!("couldn't create a hemoroid #{n}"));
}

pub async fn create_hemoroid(db: &Pool<Postgres>) {
    let (chat_id, hemoroids_repo) = get_chat_id_and_hemoroids(db);
    hemoroids_repo.create_or_grow(UserId(UID as u64), &chat_id.into(), 0)
        .await
        .expect("couldn't create a hemoroid");
}

pub async fn check_hemoroid(db: &Pool<Postgres>, protrusion_level: u32) {
    let (chat_id, hemoroids_repo) = get_chat_id_and_hemoroids(db);
    let top = hemoroids_repo.get_top(&chat_id, 0, 2)
        .await.expect("couldn't fetch the top");
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].protrusion_level, protrusion_level as i32);
    assert_eq!(top[0].owner_name, NAME);
}

async fn check_top(hemoroids_repo: &repo::Hemoroids, chat_id: &ChatIdKind, protrusion_level: i32) {
    let d = hemoroids_repo.get_top(chat_id, 0, 1)
        .await.expect("couldn't fetch the top again");
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].protrusion_level, protrusion_level);
    assert_eq!(d[0].owner_uid.0, UID);
    assert_eq!(d[0].owner_name, NAME);
}

// Helper function used in other test modules, needs to be updated
// For example, in src/repo/test/pvpstats.rs
// pub(crate) fn get_chat_id_and_dicks(db: &Pool<Postgres>) -> (ChatIdKind, repo::Dicks)
// needs to become get_chat_id_and_hemoroids and return repo::Hemoroids
// This change should be done in a separate step when updating src/repo/test/mod.rs and dependent files.
// For now, let's just fix the local helper function name if it's used here.
// It seems it's `get_chat_id_and_dicks` that needs to be locally renamed or its call sites updated.
// The definition of `get_chat_id_and_dicks` is not in this file, but it's used.
// Let's find its usage in this file and update it.
// Ah, `get_chat_id_and_dicks` is defined in `src/repo/test/mod.rs` (based on typical Rust project structure and its usage).
// I will update the call to `get_chat_id_and_dicks` here.
// And also update the `create_dick`, `check_dick` functions.
// The helper `get_chat_id_and_dicks` itself will be updated when `src/repo/test/mod.rs` is handled.

// Renaming the helper function that is likely defined in `src/repo/test/mod.rs`
// This is a placeholder for the actual change in `src/repo/test/mod.rs`
// but the functions in this file call it.
// So, I will change the calls here.

// The actual function `get_chat_id_and_dicks` is in `src/repo/test/mod.rs`.
// I'll update the calls to `get_chat_id_and_hemoroids` in this file.
// And the function names `create_dick` to `create_hemoroid`, `check_dick` to `check_hemoroid`.
// The `get_chat_id_and_dicks` in `src/repo/test/mod.rs` will be handled in Step 5.

// The change to `get_chat_id_and_dicks` to `get_chat_id_and_hemoroids`
// is actually within `src/repo/test/mod.rs`, which exports it.
// Here, I only need to change the *usage*.
// The functions `create_dick` and `check_dick` in this file need to be renamed.
// And their internal calls to `get_chat_id_and_dicks` need to change to `get_chat_id_and_hemoroids`.

// In `create_hemoroid` (formerly `create_dick`):
// `let (chat_id, dicks) = get_chat_id_and_dicks(db);` -> `let (chat_id, hemoroids_repo) = get_chat_id_and_hemoroids(db);`
// `dicks.create_or_grow` -> `hemoroids_repo.create_or_grow`

// In `check_hemoroid` (formerly `check_dick`):
// `let (chat_id, dicks) = get_chat_id_and_dicks(db);` -> `let (chat_id, hemoroids_repo) = get_chat_id_and_hemoroids(db);`
// `dicks.get_top` -> `hemoroids_repo.get_top`

// The variable `dicks` in `get_chat_id_and_dicks(db)` also needs renaming to `hemoroids_repo` for consistency.
// The function `get_chat_id_and_dicks` is defined in `src/repo/test/mod.rs`.
// The current file `src/repo/test/hemoroids.rs` *uses* this function.
// So I'll adjust the usage here. The definition will be changed in a later step.
// The `get_chat_id_and_dicks` function is not present in the provided `read_files` output for `src/repo/test/hemoroids.rs`.
// It is likely imported from `super::*` or `crate::repo::test::*`.
// The functions `create_dick` and `check_dick` are helper functions within this test file itself.
// I need to update these helper functions.

// The functions `create_dick` and `check_dick` use a helper `get_chat_id_and_dicks(db)`
// which is *not* defined in this file. It must be imported from `src/repo/test/mod.rs`.
// I will rename `create_dick` to `create_hemoroid` and `check_dick` to `check_hemoroid`.
// Inside them, I will change `get_chat_id_and_dicks` to `get_chat_id_and_hemoroids`.
// And the variable name `dicks` to `hemoroids_repo`.
// This assumes `get_chat_id_and_hemoroids` will be the new name of the function in `src/repo/test/mod.rs`.

// Let's adjust `create_dick` and `check_dick` functions.
// `create_dick` -> `create_hemoroid`
//   `get_chat_id_and_dicks` -> `get_chat_id_and_hemoroids`
//   `dicks.create_or_grow` -> `hemoroids_repo.create_or_grow`
// `check_dick` -> `check_hemoroid`
//   `get_chat_id_and_dicks` -> `get_chat_id_and_hemoroids`
//   `dicks.get_top` -> `hemoroids_repo.get_top`
//   `length` parameter -> `protrusion_level`
//   `top[0].length` -> `top[0].protrusion_level`

// The context for `get_chat_id_and_dicks` is:
// `use crate::repo::test::{CHAT_ID, get_chat_id_and_dicks, NAME, start_postgres, UID};`
// This means it's imported from `crate::repo::test`, which resolves to `src/repo/test/mod.rs`.
// So, the call must be changed to `get_chat_id_and_hemoroids`.
// And the function itself in `mod.rs` must be renamed later.

// The functions `create_user_and_dick_2` and `create_another_user_and_dick` also need similar updates.
// `create_user_and_dick_2` -> `create_user_and_hemoroid_2`
//   `create_another_user_and_dick` -> `create_another_user_and_hemoroid`
// `create_another_user_and_dick` -> `create_another_user_and_hemoroid`
//   `repo::Dicks::new` -> `repo::Hemoroids::new` (variable `dicks` to `hemoroids_repo`)
//   `dicks.create_or_grow` -> `hemoroids_repo.create_or_grow`
//   "couldn't create a dick" -> "couldn't create a hemoroid"
