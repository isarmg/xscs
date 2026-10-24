use super::*;
use serde_json::json;

fn validate(rules: Value, value: Value) -> std::result::Result<(), ConfigError> {
    validate_rules(
        &rules,
        &value,
        Path::new("/opt/fixture"),
        ConfigSource::File,
    )
}

#[test]
fn current_numeric_socket_and_enum_rules_accept_boundaries_and_reject_unsafe_values() {
    let rules = json!([
        {"kind":"integer-range","fields":["/idle"],"arguments":{"minimum":1,"maximum":60}},
        {"kind":"integer-order","fields":["/idle","/total"],"arguments":{"relation":"less-or-equal"}},
        {"kind":"enum","fields":["/mode"],"arguments":{"values":["off","on"]}},
        {"kind":"socket-address","fields":["/bind"],"arguments":{"loopback_when":"/development","nonzero":true}},
        {"kind":"distinct-sockets","fields":["/bind","/callback"],"arguments":{}}
    ]);
    let valid = json!({"idle":1,"total":60,"mode":"on","bind":"[::1]:8080","callback":"127.0.0.1:8080","development":true});
    validate(rules.clone(), valid.clone()).unwrap();
    for (field, bad) in [
        ("idle", json!(0)),
        ("total", json!(0)),
        ("mode", json!("unknown")),
        ("bind", json!("0.0.0.0:8080")),
        ("callback", json!("[::1]:8080")),
        ("callback", json!("0.0.0.0:8080")),
        ("callback", json!("[::]:8080")),
    ] {
        let mut value = valid.clone();
        value[field] = bad;
        let error = validate(rules.clone(), value).unwrap_err();
        assert_eq!(error.source, ConfigSource::File);
        assert!(!serde_json::to_string(&error).unwrap().contains("unknown"));
    }
    let mut maximum = valid;
    maximum["idle"] = json!(60);
    validate(rules, maximum).unwrap();
}

#[test]
fn state_paths_resolve_fallbacks_and_reject_reserved_or_overlapping_namespaces() {
    let paths = json!([
        {"base":"config","value":"/data","fallback":{"base":"root","value":"state","fallback":null,"suffix":"","encoding":"path"},"suffix":"","encoding":"path"},
        {"base":"config","value":"/media","fallback":null,"suffix":"","encoding":"path"}
    ]);
    let rules = json!([{"kind":"root-state-paths","fields":[],"arguments":{"paths":paths,"protected":crate::server_layout::ROOT_RESERVED_DIRECTORIES}}]);
    validate(rules.clone(), json!({"data":null,"media":"/srv/video"})).unwrap();
    for media in [
        "/opt/fixture/bin/videos",
        "/opt/fixture",
        "/opt/fixture/state/sub",
        "/opt/fixture/../escape",
        "/",
    ] {
        assert!(validate(rules.clone(), json!({"data":null,"media":media})).is_err());
    }
    let sqlite = json!([{"kind":"root-state-paths","fields":[],"arguments":{"paths":[{"base":"config","value":"/database","fallback":null,"suffix":"","encoding":"sqlite-url"}],"protected":crate::server_layout::ROOT_RESERVED_DIRECTORIES}}]);
    validate(
        sqlite.clone(),
        json!({"database":"sqlite:///srv/data/current.db"}),
    )
    .unwrap();
    assert!(
        validate(
            sqlite,
            json!({"database":"sqlite:///srv/data/current.db?mode=rw"})
        )
        .is_err()
    );
}

#[test]
fn url_rules_match_loopback_and_current_production_rtsp_policy() {
    let rules = json!([{"kind":"url","fields":["/url"],"arguments":{"allowed_schemes":["rtsp","rtsps"],"no_credentials":true,"no_query":true,"no_fragment":true,"origin_only":true,"production_public_rtsps":true,"development_pointer":"/development"}}]);
    for valid in ["rtsps://video.example:8322", "rtsps://192.168.1.20:8322"] {
        validate(rules.clone(), json!({"url":valid,"development":false})).unwrap();
    }
    for invalid in [
        "rtsp://video.example:8554",
        "rtsps://[::1]:8322",
        "rtsps://0.0.0.0:8322",
        "rtsps://localhost:8322",
        "rtsps://user:pass@video.example:8322",
        "rtsps://video.example:8322/camera",
        "rtsps://video.example:8322?secret=private",
    ] {
        let error =
            validate(rules.clone(), json!({"url":invalid,"development":false})).unwrap_err();
        assert_eq!(error.path, "/url");
        assert!(!error.to_string().contains("private"));
    }
    validate(rules, json!({"url":"rtsp://[::1]:8554","development":true})).unwrap();
    let loopback = json!([{"kind":"url","fields":["/url"],"arguments":{"allowed_schemes":["http"],"loopback_host":true,"no_credentials":true,"no_query":true,"no_fragment":true}}]);
    validate(loopback.clone(), json!({"url":"http://[::1]:9997"})).unwrap();
    assert!(validate(loopback, json!({"url":"http://example.org:9997"})).is_err());
}

#[test]
fn external_assets_require_explicit_development_and_unbound_source() {
    for bound in [false, true] {
        let rule = json!([{"kind":"external-web-override","fields":["/web"],"arguments":{"development_pointer":"/development","source_bound":bound}}]);
        for development in [false, true] {
            validate(rule.clone(), json!({"web":null,"development":development})).unwrap();
            assert_eq!(
                validate(
                    rule.clone(),
                    json!({"web":"/tmp/assets","development":development})
                )
                .is_ok(),
                development && !bound
            );
        }
    }
}

#[test]
fn unknown_rules_arguments_missing_values_and_definition_bounds_fail_closed() {
    for rule in [
        json!({"kind":"future-rule","fields":[],"arguments":{}}),
        json!({"kind":"path","fields":["/path"],"arguments":{"allow_unsafe":true}}),
        json!({"kind":"integer-range","fields":["/x"],"arguments":{"minimum":10,"maximum":1}}),
        json!({"kind":"path","fields":["/x~9"],"arguments":{}}),
    ] {
        assert_eq!(
            validate(json!([rule]), json!({"path":"/tmp"}))
                .unwrap_err()
                .reason,
            Reason::InvalidMapping
        );
    }
    let missing = validate(
        json!([{"kind":"path","fields":["/missing"],"arguments":{}}]),
        json!({}),
    )
    .unwrap_err();
    assert_eq!(missing.reason, Reason::MissingField);
    let oversized = Value::Array(vec![
        json!({"kind":"path","fields":["/path"],"arguments":{}});
        MAX_RULES + 1
    ]);
    assert!(validate(oversized, json!({"path":"/tmp"})).is_err());
}

#[test]
fn declared_path_relationship_preserves_allowed_subtrees_and_protects_private_entries() {
    let spec = |pointer| json!({"base":"config","value":pointer,"fallback":null,"suffix":"","encoding":"path"});
    for allow in [false, true] {
        let rules = json!([{"kind":"path-relation","fields":[],"arguments":{"parent":spec("/private"),"child":spec("/business"),"allow_child_subtree":allow,"forbidden_child_first_component_prefixes":["current.db","secrets.json",".state-"]}}]);
        validate(
            rules.clone(),
            json!({"private":"/srv/private","business":"/srv/business"}),
        )
        .unwrap();
        assert_eq!(
            validate(
                rules.clone(),
                json!({"private":"/srv/private","business":"/srv/private/media"})
            )
            .is_ok(),
            allow
        );
        for business in [
            "/srv/private",
            "/srv",
            "/srv/private/current.db-wal",
            "/srv/private/secrets.json/sub",
            "/srv/private/.state-instance.lock",
        ] {
            assert!(
                validate(
                    rules.clone(),
                    json!({"private":"/srv/private","business":business})
                )
                .is_err()
            );
        }
    }
}

#[test]
fn integer_order_supports_strict_unit_conversion_and_rejects_overflow_or_unknown_arguments() {
    let rule = |arguments| json!([{"kind":"integer-order","fields":["/left","/right"],"arguments":arguments}]);
    let strict = rule(json!({"relation":"less-than","right_scale":1000}));
    validate(strict.clone(), json!({"left":999,"right":1})).unwrap();
    for left in [1000, 1001, u64::MAX] {
        assert!(validate(strict.clone(), json!({"left":left,"right":1})).is_err());
    }
    assert!(validate(strict, json!({"left":1,"right":u64::MAX})).is_err());
    let inclusive = rule(json!({"relation":"less-or-equal","left_scale":1000}));
    validate(inclusive.clone(), json!({"left":1,"right":1000})).unwrap();
    assert!(validate(inclusive, json!({"left":1,"right":999})).is_err());
    validate(
        rule(json!({"relation":"less-or-equal"})),
        json!({"left":1,"right":1}),
    )
    .unwrap();
    assert!(
        validate(
            rule(json!({"relation":"less-than"})),
            json!({"left":1,"right":1})
        )
        .is_err()
    );
    for args in [
        json!({"relation":"less-than","left_scale":0}),
        json!({"relation":"less-than","right_scale":-1}),
        json!({"relation":"less-than","left_scale":true}),
        json!({"relation":"less-than","unrecognized":1}),
    ] {
        assert_eq!(
            validate(rule(args), json!({"left":1,"right":2}))
                .unwrap_err()
                .reason,
            Reason::InvalidMapping
        );
    }
}
