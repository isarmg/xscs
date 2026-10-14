//! Versioned configuration field policy shared by protocol validation and the Manager UI.
use crate::{InvalidTask, SUPPORTED_SUNSHINE_VERSIONS};
use serde::{Deserialize, Serialize};

const ALL_OPERATING_SYSTEMS: &[&str] = &[
    "linux_x86_64",
    "windows_x86_64",
    "macos_x86_64",
    "macos_aarch64",
];
const DESKTOP_NVIDIA_OPERATING_SYSTEMS: &[&str] = &["linux_x86_64", "windows_x86_64"];
const WINDOWS: &[&str] = &["windows_x86_64"];
const LINUX: &[&str] = &["linux_x86_64"];
const MACOS: &[&str] = &["macos_x86_64", "macos_aarch64"];
const LOG_LEVELS: &[&str] = &[
    "verbose", "debug", "info", "warning", "error", "fatal", "none", "0", "1", "2", "3", "4", "5",
    "6",
];
const SOFTWARE_PRESETS: &[&str] = &[
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
];
const SOFTWARE_TUNES: &[&str] = &[
    "film",
    "animation",
    "grain",
    "stillimage",
    "fastdecode",
    "zerolatency",
    "psnr",
    "ssim",
];
const ENCODERS: &[&str] = &[
    "nvenc",
    "quicksync",
    "amdvce",
    "software",
    "vaapi",
    "vulkan",
    "videotoolbox",
];
const NVENC_TWOPASS: &[&str] = &["disabled", "quarter_res", "full_res"];
const NVENC_SPLIT: &[&str] = &["disabled", "driver_decides", "enabled"];
const CODERS: &[&str] = &["auto", "cabac", "cavlc"];
const QSV_PRESETS: &[&str] = &[
    "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow",
];
const AMD_USAGE: &[&str] = &[
    "transcoding",
    "webcam",
    "lowlatency_high_quality",
    "lowlatency",
    "ultralowlatency",
];
const AMD_RATE_CONTROL: &[&str] = &["cqp", "cbr", "vbr_latency", "vbr_peak"];
const QUALITY: &[&str] = &["speed", "balanced", "quality"];
const VAAPI_QUALITY: &[&str] = &["auto", "speed", "balanced", "quality"];
const VAAPI_RATE_CONTROL: &[&str] = &["auto", "avbr", "cbr", "cqp", "icq", "qvbr", "vbr"];
const VT_SOFTWARE: &[&str] = &["auto", "disabled", "allowed", "forced"];
const GAMEPAD_DRIVERS: &[&str] = &["all", "virtualhid", "vigembus"];
const GAMEPADS: &[&str] = &[
    "auto", "generic", "ds4", "ds5", "switch", "x360", "xone", "xseries",
];
const DISPLAY_CONFIGURATION: &[&str] = &[
    "disabled",
    "verify_only",
    "ensure_active",
    "ensure_primary",
    "ensure_only_display",
];
const DISPLAY_VALUE_MODE: &[&str] = &["disabled", "auto", "manual"];
const DISPLAY_HDR_MODE: &[&str] = &["disabled", "auto"];
const LOCALES: &[&str] = &[
    "bg", "cs", "de", "en", "en_GB", "en_US", "es", "fr", "hu", "it", "ja", "ko", "pl", "pt",
    "pt_BR", "ru", "sv", "tr", "uk", "vi", "zh", "zh_TW",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Integer,
    Boolean,
    Select,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FieldDefinition {
    pub key: &'static str,
    pub kind: FieldKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_length: Option<usize>,
    #[serde(skip_serializing_if = "slice_is_empty")]
    pub options: &'static [&'static str],
    pub requires_restart: bool,
    pub supported_sunshine_versions: &'static [&'static str],
    pub operating_systems: &'static [&'static str],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prerequisite: Option<&'static str>,
}

const fn slice_is_empty(value: &&[&str]) -> bool {
    value.is_empty()
}

const fn field(
    key: &'static str,
    kind: FieldKind,
    range: Option<(i64, i64)>,
    maximum_length: Option<usize>,
    options: &'static [&'static str],
    operating_systems: &'static [&'static str],
    prerequisite: Option<&'static str>,
) -> FieldDefinition {
    FieldDefinition {
        key,
        kind,
        minimum: match range {
            Some((minimum, _)) => Some(minimum),
            None => None,
        },
        maximum: match range {
            Some((_, maximum)) => Some(maximum),
            None => None,
        },
        maximum_length,
        options,
        requires_restart: true,
        supported_sunshine_versions: SUPPORTED_SUNSHINE_VERSIONS,
        operating_systems,
        prerequisite,
    }
}

pub const FIELD_DEFINITIONS: &[FieldDefinition] = &[
    field(
        "sunshine_name",
        FieldKind::Text,
        None,
        Some(32),
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "min_log_level",
        FieldKind::Select,
        None,
        None,
        LOG_LEVELS,
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "qp",
        FieldKind::Integer,
        Some((0, 51)),
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "hevc_mode",
        FieldKind::Integer,
        Some((0, 3)),
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "av1_mode",
        FieldKind::Integer,
        Some((0, 3)),
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "min_threads",
        FieldKind::Integer,
        Some((1, 64)),
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        Some("software_encoder"),
    ),
    field(
        "sw_preset",
        FieldKind::Select,
        None,
        None,
        SOFTWARE_PRESETS,
        ALL_OPERATING_SYSTEMS,
        Some("software_encoder"),
    ),
    field(
        "nvenc_preset",
        FieldKind::Integer,
        Some((1, 7)),
        None,
        &[],
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("nvidia_encoder"),
    ),
    field(
        "nvenc_vbv_increase",
        FieldKind::Integer,
        Some((0, 400)),
        None,
        &[],
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("nvidia_encoder"),
    ),
    field(
        "nvenc_spatial_aq",
        FieldKind::Boolean,
        None,
        None,
        &[],
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("nvidia_encoder"),
    ),
    field(
        "nvenc_h264_cavlc",
        FieldKind::Boolean,
        None,
        None,
        &[],
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("nvidia_encoder"),
    ),
    field(
        "locale",
        FieldKind::Select,
        None,
        None,
        LOCALES,
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "notify_pre_releases",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "system_tray",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        Some("interactive_desktop"),
    ),
    field(
        "encoder",
        FieldKind::Select,
        None,
        None,
        ENCODERS,
        ALL_OPERATING_SYSTEMS,
        Some("installed_encoder"),
    ),
    field(
        "fec_percentage",
        FieldKind::Integer,
        Some((1, 255)),
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "max_bitrate",
        FieldKind::Integer,
        Some((0, 1_000_000)),
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "stream_audio",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "controller",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "keyboard",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "mouse",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "always_send_scancodes",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "key_rightalt_to_key_win",
        FieldKind::Boolean,
        None,
        None,
        &[],
        WINDOWS,
        None,
    ),
    field(
        "high_resolution_scrolling",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "native_pen_touch",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "back_button_timeout",
        FieldKind::Integer,
        Some((-1, 60_000)),
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "key_repeat_delay",
        FieldKind::Integer,
        Some((0, 60_000)),
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        None,
    ),
    field(
        "gamepad_driver",
        FieldKind::Select,
        None,
        None,
        GAMEPAD_DRIVERS,
        WINDOWS,
        Some("installed_gamepad_driver"),
    ),
    field(
        "gamepad",
        FieldKind::Select,
        None,
        None,
        GAMEPADS,
        ALL_OPERATING_SYSTEMS,
        Some("installed_gamepad_driver"),
    ),
    field(
        "ds4_back_as_touchpad_click",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        Some("ds4_emulation"),
    ),
    field(
        "motion_as_ds4",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        Some("ds4_emulation"),
    ),
    field(
        "touchpad_as_ds4",
        FieldKind::Boolean,
        None,
        None,
        &[],
        ALL_OPERATING_SYSTEMS,
        Some("ds4_emulation"),
    ),
    field(
        "virtualhid_randomize_mac",
        FieldKind::Boolean,
        None,
        None,
        &[],
        WINDOWS,
        Some("virtualhid_driver"),
    ),
    field(
        "dd_configuration_option",
        FieldKind::Select,
        None,
        None,
        DISPLAY_CONFIGURATION,
        WINDOWS,
        Some("windows_display_control"),
    ),
    field(
        "dd_resolution_option",
        FieldKind::Select,
        None,
        None,
        DISPLAY_VALUE_MODE,
        WINDOWS,
        Some("windows_display_control"),
    ),
    field(
        "dd_manual_resolution",
        FieldKind::Text,
        None,
        Some(32),
        &[],
        WINDOWS,
        Some("manual_resolution"),
    ),
    field(
        "dd_refresh_rate_option",
        FieldKind::Select,
        None,
        None,
        DISPLAY_VALUE_MODE,
        WINDOWS,
        Some("windows_display_control"),
    ),
    field(
        "dd_manual_refresh_rate",
        FieldKind::Text,
        None,
        Some(32),
        &[],
        WINDOWS,
        Some("manual_refresh_rate"),
    ),
    field(
        "dd_hdr_option",
        FieldKind::Select,
        None,
        None,
        DISPLAY_HDR_MODE,
        WINDOWS,
        Some("windows_display_control"),
    ),
    field(
        "dd_wa_hdr_toggle_delay",
        FieldKind::Integer,
        Some((0, 3000)),
        None,
        &[],
        WINDOWS,
        Some("windows_display_control"),
    ),
    field(
        "dd_config_revert_delay",
        FieldKind::Integer,
        Some((0, i32::MAX as i64)),
        None,
        &[],
        WINDOWS,
        Some("windows_display_control"),
    ),
    field(
        "dd_config_revert_on_disconnect",
        FieldKind::Boolean,
        None,
        None,
        &[],
        WINDOWS,
        Some("windows_display_control"),
    ),
    field(
        "sw_tune",
        FieldKind::Select,
        None,
        None,
        SOFTWARE_TUNES,
        ALL_OPERATING_SYSTEMS,
        Some("software_encoder"),
    ),
    field(
        "nvenc_twopass",
        FieldKind::Select,
        None,
        None,
        NVENC_TWOPASS,
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("nvidia_encoder"),
    ),
    field(
        "nvenc_split_encode",
        FieldKind::Select,
        None,
        None,
        NVENC_SPLIT,
        WINDOWS,
        Some("nvidia_encoder"),
    ),
    field(
        "nvenc_realtime_hags",
        FieldKind::Boolean,
        None,
        None,
        &[],
        WINDOWS,
        Some("nvidia_encoder"),
    ),
    field(
        "nvenc_opengl_vulkan_on_dxgi",
        FieldKind::Boolean,
        None,
        None,
        &[],
        WINDOWS,
        Some("nvidia_encoder"),
    ),
    field(
        "nvenc_latency_over_power",
        FieldKind::Boolean,
        None,
        None,
        &[],
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("nvidia_encoder"),
    ),
    field(
        "qsv_preset",
        FieldKind::Select,
        None,
        None,
        QSV_PRESETS,
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("intel_encoder"),
    ),
    field(
        "qsv_coder",
        FieldKind::Select,
        None,
        None,
        CODERS,
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("intel_encoder"),
    ),
    field(
        "qsv_slow_hevc",
        FieldKind::Boolean,
        None,
        None,
        &[],
        DESKTOP_NVIDIA_OPERATING_SYSTEMS,
        Some("intel_encoder"),
    ),
    field(
        "amd_usage",
        FieldKind::Select,
        None,
        None,
        AMD_USAGE,
        WINDOWS,
        Some("amd_encoder"),
    ),
    field(
        "amd_rc",
        FieldKind::Select,
        None,
        None,
        AMD_RATE_CONTROL,
        WINDOWS,
        Some("amd_encoder"),
    ),
    field(
        "amd_quality",
        FieldKind::Select,
        None,
        None,
        QUALITY,
        WINDOWS,
        Some("amd_encoder"),
    ),
    field(
        "amd_coder",
        FieldKind::Select,
        None,
        None,
        CODERS,
        WINDOWS,
        Some("amd_encoder"),
    ),
    field(
        "amd_preanalysis",
        FieldKind::Boolean,
        None,
        None,
        &[],
        WINDOWS,
        Some("amd_encoder"),
    ),
    field(
        "amd_vbaq",
        FieldKind::Boolean,
        None,
        None,
        &[],
        WINDOWS,
        Some("amd_encoder"),
    ),
    field(
        "amd_enforce_hrd",
        FieldKind::Boolean,
        None,
        None,
        &[],
        WINDOWS,
        Some("amd_encoder"),
    ),
    field(
        "amd_max_au_size",
        FieldKind::Integer,
        Some((-1, i32::MAX as i64)),
        None,
        &[],
        WINDOWS,
        Some("amd_encoder"),
    ),
    field(
        "vaapi_quality",
        FieldKind::Select,
        None,
        None,
        VAAPI_QUALITY,
        LINUX,
        Some("vaapi_encoder"),
    ),
    field(
        "vaapi_rc",
        FieldKind::Select,
        None,
        None,
        VAAPI_RATE_CONTROL,
        LINUX,
        Some("vaapi_encoder"),
    ),
    field(
        "vaapi_blbrc",
        FieldKind::Boolean,
        None,
        None,
        &[],
        LINUX,
        Some("vaapi_encoder"),
    ),
    field(
        "vaapi_strict_rc_buffer",
        FieldKind::Boolean,
        None,
        None,
        &[],
        LINUX,
        Some("vaapi_encoder"),
    ),
    field(
        "vt_coder",
        FieldKind::Select,
        None,
        None,
        CODERS,
        MACOS,
        Some("videotoolbox_encoder"),
    ),
    field(
        "vt_software",
        FieldKind::Select,
        None,
        None,
        VT_SOFTWARE,
        MACOS,
        Some("videotoolbox_encoder"),
    ),
    field(
        "vt_realtime",
        FieldKind::Boolean,
        None,
        None,
        &[],
        MACOS,
        Some("videotoolbox_encoder"),
    ),
];

pub fn field_definition(key: &str) -> Option<&'static FieldDefinition> {
    FIELD_DEFINITIONS.iter().find(|field| field.key == key)
}

pub fn contains_field(key: &str) -> bool {
    field_definition(key).is_some()
}

pub fn normalized_snapshot_value(key: &str, value: &str) -> Option<String> {
    let definition = field_definition(key)?;
    match definition.kind {
        FieldKind::Boolean => match value.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" | "enabled" => Some("true".into()),
            "false" | "0" | "no" | "off" | "disabled" => Some("false".into()),
            _ => None,
        },
        FieldKind::Integer => value.parse::<i64>().ok().map(|value| value.to_string()),
        FieldKind::Text | FieldKind::Select => Some(value.to_owned()),
    }
}

pub fn validate_snapshot_field(key: &str, value: &str) -> Result<(), InvalidTask> {
    let definition = field_definition(key).ok_or(InvalidTask)?;
    let value = normalized_snapshot_value(key, value).ok_or(InvalidTask)?;
    match definition.kind {
        FieldKind::Boolean => validate_field(key, &FieldValue::Boolean(value == "true")),
        FieldKind::Integer => validate_field(
            key,
            &FieldValue::Integer(value.parse().map_err(|_| InvalidTask)?),
        ),
        FieldKind::Text | FieldKind::Select => validate_field(key, &FieldValue::Text(value)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum FieldValue {
    Boolean(bool),
    Integer(i64),
    Text(String),
}

impl FieldValue {
    pub fn sunshine_text(&self) -> String {
        match self {
            Self::Boolean(value) => value.to_string(),
            Self::Integer(value) => value.to_string(),
            Self::Text(value) => value.clone(),
        }
    }
}

pub fn validate_field(key: &str, value: &FieldValue) -> Result<(), InvalidTask> {
    let definition = field_definition(key).ok_or(InvalidTask)?;
    let valid = match (definition.kind, value) {
        (FieldKind::Text, FieldValue::Text(value)) => {
            !value.trim().is_empty()
                && value == value.trim()
                && definition
                    .maximum_length
                    .is_none_or(|maximum| value.chars().count() <= maximum)
                && !value.chars().any(|character| {
                    character.is_control() || matches!(character, '#' | '=' | '[' | ']')
                })
        }
        (FieldKind::Integer, FieldValue::Integer(value)) => {
            definition.minimum.is_none_or(|minimum| *value >= minimum)
                && definition.maximum.is_none_or(|maximum| *value <= maximum)
        }
        (FieldKind::Boolean, FieldValue::Boolean(_)) => true,
        (FieldKind::Select, FieldValue::Text(value)) => {
            definition.options.contains(&value.as_str())
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(InvalidTask) }
}
