#[path = "support/database.rs"]
mod fixture_database;
use xscs::{
    access::{self, UpdateAccessRules},
    crypto::SecretBox,
    db,
};
use xscs_protocol::*;

async fn create(
    pool: &sqlx::SqlitePool,
    role: InstanceRole,
    name: &str,
    enroll: bool,
) -> (String, String) {
    let ticket = db::create_device(
        pool,
        &SecretBox::new("test", [7; 32]).unwrap(),
        name,
        role,
        "admin",
    )
    .await
    .unwrap();
    let credential = db::random_token();
    if enroll {
        db::enroll(
            pool,
            &ticket.device.id,
            uuid::Uuid::new_v4(),
            &ticket.token,
            &credential,
        )
        .await
        .unwrap();
    }
    (ticket.device.id, credential)
}
async fn replace(
    pool: &sqlx::SqlitePool,
    id: &str,
    peer_ids: &[&str],
) -> xscs::AppResult<access::AccessRules> {
    let current = access::rules(pool, id).await?;
    access::replace_rules(
        pool,
        id,
        UpdateAccessRules {
            peer_ids: peer_ids.iter().map(|s| (*s).to_owned()).collect(),
            expected_revision: current.revision,
        },
        "admin",
    )
    .await
}
async fn fixture() -> (tempfile::TempDir, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = fixture_database::open_or_initialize(&format!(
        "sqlite://{}",
        dir.path().join("state.db").display()
    ))
    .await
    .unwrap();
    (dir, pool)
}

#[tokio::test]
async fn directed_permissions_are_one_relation_and_updates_are_atomic() {
    let (_dir, pool) = fixture().await;
    let (controller, _) = create(&pool, InstanceRole::Controller, "Controller", true).await;
    let (host, _) = create(&pool, InstanceRole::Controlled, "Host", true).await;
    let (other, _) = create(&pool, InstanceRole::Controller, "Other controller", true).await;
    let stale = access::rules(&pool, &host).await.unwrap();
    replace(&pool, &controller, &[&host]).await.unwrap();
    assert_eq!(
        access::rules(&pool, &host).await.unwrap().peer_ids,
        std::slice::from_ref(&controller)
    );
    assert!(
        access::replace_rules(
            &pool,
            &host,
            UpdateAccessRules {
                peer_ids: vec![],
                expected_revision: stale.revision
            },
            "admin"
        )
        .await
        .is_err()
    );
    for invalid in [
        vec![controller.as_str()],
        vec![other.as_str()],
        vec![host.as_str(), host.as_str()],
        vec![host.as_str(), "missing"],
    ] {
        assert!(replace(&pool, &controller, &invalid).await.is_err());
        assert_eq!(
            access::rules(&pool, &controller).await.unwrap().peer_ids,
            std::slice::from_ref(&host)
        );
    }
    replace(&pool, &host, &[&other]).await.unwrap();
    assert!(
        access::rules(&pool, &controller)
            .await
            .unwrap()
            .peer_ids
            .is_empty()
    );
    assert_eq!(
        access::rules(&pool, &other).await.unwrap().peer_ids,
        std::slice::from_ref(&host)
    );
    assert!(
        sqlx::query("INSERT INTO device_access(controller_id,controlled_id) VALUES(?,?)")
            .bind(&host)
            .bind(&other)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE devices SET role='controlled' WHERE device_id=?")
            .bind(&controller)
            .execute(&pool)
            .await
            .is_err()
    );
    db::delete_device(&pool, &host, "admin").await.unwrap();
    assert!(
        access::rules(&pool, &other)
            .await
            .unwrap()
            .peer_ids
            .is_empty()
    );
}

#[tokio::test]
async fn clients_get_only_bound_allowed_peers_and_fresh_host_addresses() {
    let (_dir, pool) = fixture().await;
    let (controller, token) = create(&pool, InstanceRole::Controller, "Controller", true).await;
    let (host, _) = create(&pool, InstanceRole::Controlled, "Host", true).await;
    let (unbound, _) = create(&pool, InstanceRole::Controlled, "Unbound", false).await;
    let (_unrelated, _) = create(&pool, InstanceRole::Controlled, "Unrelated", true).await;
    let own = db::authenticate_device(&pool, &token).await.unwrap();
    assert!(
        access::policy(&pool, &own, None)
            .await
            .unwrap()
            .peers
            .is_empty()
    );
    replace(&pool, &controller, &[&host, &unbound])
        .await
        .unwrap();
    let host_device = db::get_device(&pool, &host).await.unwrap();
    let binding = db::binding(&pool, &host_device).await.unwrap();
    sqlx::query(
        "UPDATE devices SET session_id='host-session',last_seen_at_micros=? WHERE device_id=?",
    )
    .bind(db::now_micros().unwrap())
    .bind(&host)
    .execute(&pool)
    .await
    .unwrap();
    let addresses = vec![
        HostAddress {
            host: "host.example.com".into(),
            port: 21118,
        },
        HostAddress {
            host: "2001:db8::10".into(),
            port: 21118,
        },
    ];
    assert!(
        access::report_addresses(&pool, &binding, "superseded", &addresses)
            .await
            .is_err()
    );
    access::report_addresses(&pool, &binding, "host-session", &addresses)
        .await
        .unwrap();
    let policy = access::policy(&pool, &own, None).await.unwrap();
    assert_eq!(policy.peers.len(), 1);
    assert_eq!(policy.peers[0].binding, binding);
    assert_eq!(policy.peers[0].addresses, addresses);
    assert!(
        access::report_addresses(
            &pool,
            &db::binding(&pool, &own).await.unwrap(),
            "controller-session",
            &addresses
        )
        .await
        .is_err()
    );
    let host_policy = access::policy(
        &pool,
        &db::get_device(&pool, &host).await.unwrap(),
        Some("host-session"),
    )
    .await
    .unwrap();
    assert_eq!(host_policy.role, InstanceRole::Controlled);
    assert_eq!(
        host_policy.peers[0].binding.device_id.to_string(),
        controller
    );
    assert!(host_policy.peers[0].addresses.is_empty());
    sqlx::query("UPDATE devices SET addresses_at_micros=? WHERE device_id=?")
        .bind(db::now_micros().unwrap() - 46_000_000)
        .bind(&host)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        access::policy(&pool, &own, None).await.unwrap().peers[0]
            .addresses
            .is_empty()
    );
    access::report_addresses(&pool, &binding, "host-session", &[])
        .await
        .unwrap();
    assert!(
        db::get_device(&pool, &host)
            .await
            .unwrap()
            .view(db::now_micros().unwrap())
            .unwrap()
            .addresses
            .is_empty()
    );
    db::revoke(&pool, &host, "admin").await.unwrap();
    assert!(
        access::policy(&pool, &own, None)
            .await
            .unwrap()
            .peers
            .is_empty()
    );
    db::rotate_authorization(
        &pool,
        &SecretBox::new("test", [7; 32]).unwrap(),
        &controller,
        &"b".repeat(36),
        "admin",
    )
    .await
    .unwrap();
    assert!(access::policy(&pool, &own, None).await.is_err());
}

#[tokio::test]
async fn peer_capacity_is_checked_from_both_sides() {
    let (_dir, pool) = fixture().await;
    let (host, _) = create(&pool, InstanceRole::Controlled, "Host", false).await;
    for index in 0..MAX_ACCESS_PEERS {
        let (id, _) = create(
            &pool,
            InstanceRole::Controller,
            &format!("Controller {index}"),
            false,
        )
        .await;
        replace(&pool, &id, &[&host]).await.unwrap();
    }
    let (overflow, _) = create(&pool, InstanceRole::Controller, "Overflow", false).await;
    assert!(replace(&pool, &overflow, &[&host]).await.is_err());
    assert!(
        access::rules(&pool, &overflow)
            .await
            .unwrap()
            .peer_ids
            .is_empty()
    );
    assert_eq!(
        access::rules(&pool, &host).await.unwrap().peer_ids.len(),
        MAX_ACCESS_PEERS
    );
}
