use serde::Serialize;

/// Maximum serialized size for `toolInput` or `toolResult` in bytes (128 KB).
pub const MAX_PAYLOAD_SIZE: usize = 128 * 1024;

/// Hook event types.
///
/// Accepts both PascalCase (`"PreToolUse"`) and snake_case (`"pre_tool_use"`)
/// during deserialization for migration compatibility.
/// Serializes to snake_case for the hook envelope wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEventName {
    // ── Session lifecycle ───────────────────────────────────────
    SessionStart,
    SessionEnd,
    /// Fires when an agent turn ends (completed, cancelled, or error).
    Stop,
    /// Fires when the turn ends due to an API error. Output and exit code are ignored.
    StopFailure,

    // ── Tool events ─────────────────────────────────────────────
    PreToolUse,
    PostToolUse,
    /// Fires after a tool call fails (throws an error).
    PostToolUseFailure,
    /// Fires when a tool call is denied by the permission system.
    PermissionDenied,

    // ── User / notification events ──────────────────────────────
    /// Fires when the user submits a prompt.
    UserPromptSubmit,
    /// Fires when a notification is sent (e.g., permission prompt, idle).
    Notification,

    // ── Subagent events ─────────────────────────────────────────
    /// Fires when a subagent is spawned.
    SubagentStart,
    /// Fires when a subagent completes.
    SubagentStop,
    /// Alias for SubagentStop (kept for backward compatibility).
    SubagentEnd,

    // ── Compaction events ───────────────────────────────────────
    /// Fires before context compaction.
    PreCompact,
    /// Fires after context compaction completes.
    PostCompact,
}

impl<'de> serde::Deserialize<'de> for HookEventName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            // PascalCase (native) + snake_case + camelCase (third-party compat).
            // Per-operation hook names (beforeShellExecution, afterFileEdit, etc.)
            // map to our generic PreToolUse/PostToolUse — the hook script receives the
            // tool name in JSON input and can filter, or use the `matcher` field.
            "SessionStart" | "session_start" | "sessionStart" => Ok(Self::SessionStart),
            "PreToolUse"
            | "pre_tool_use"
            | "preToolUse"
            | "beforeShellExecution"
            | "beforeMCPExecution"
            | "beforeReadFile" => Ok(Self::PreToolUse),
            "PostToolUse"
            | "post_tool_use"
            | "postToolUse"
            | "afterShellExecution"
            | "afterMCPExecution"
            | "afterFileEdit"
            | "afterAgentResponse"
            | "afterAgentThought" => Ok(Self::PostToolUse),
            "PostToolUseFailure" | "post_tool_use_failure" | "postToolUseFailure" => {
                Ok(Self::PostToolUseFailure)
            }
            "SessionEnd" | "session_end" | "sessionEnd" => Ok(Self::SessionEnd),
            "Stop" | "stop" => Ok(Self::Stop),
            "StopFailure" | "stop_failure" | "stopFailure" => Ok(Self::StopFailure),
            "Notification" | "notification" => Ok(Self::Notification),
            "UserPromptSubmit" | "user_prompt_submit" | "beforeSubmitPrompt" => {
                Ok(Self::UserPromptSubmit)
            }
            "PermissionDenied" | "permission_denied" | "permissionDenied" => {
                Ok(Self::PermissionDenied)
            }
            "SubagentStart" | "subagent_start" | "subagentStart" => Ok(Self::SubagentStart),
            "SubagentStop" | "subagent_stop" | "subagentStop" => Ok(Self::SubagentStop),
            "SubagentEnd" | "subagent_end" | "subagentEnd" => Ok(Self::SubagentEnd),
            "PreCompact" | "pre_compact" | "preCompact" => Ok(Self::PreCompact),
            "PostCompact" | "post_compact" | "postCompact" => Ok(Self::PostCompact),
            other => Err(serde::de::Error::custom(format!(
                "unknown hook event: '{other}'. Expected one of: \
                 SessionStart, PreToolUse, PostToolUse, PostToolUseFailure, \
                 SessionEnd, Stop, StopFailure, Notification, UserPromptSubmit, \
                 PermissionDenied, SubagentStart, SubagentStop, \
                 PreCompact, PostCompact"
            ))),
        }
    }
}

impl std::fmt::Display for HookEventName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionStart => write!(f, "session_start"),
            Self::PreToolUse => write!(f, "pre_tool_use"),
            Self::PostToolUse => write!(f, "post_tool_use"),
            Self::PostToolUseFailure => write!(f, "post_tool_use_failure"),
            Self::SessionEnd => write!(f, "session_end"),
            Self::Stop => write!(f, "stop"),
            Self::StopFailure => write!(f, "stop_failure"),
            Self::Notification => write!(f, "notification"),
            Self::UserPromptSubmit => write!(f, "user_prompt_submit"),
            Self::PermissionDenied => write!(f, "permission_denied"),
            Self::SubagentStart => write!(f, "subagent_start"),
            Self::SubagentStop | Self::SubagentEnd => write!(f, "subagent_stop"),
            Self::PreCompact => write!(f, "pre_compact"),
            Self::PostCompact => write!(f, "post_compact"),
        }
    }
}

impl HookEventName {
    /// Collapse alias variants to their canonical form so a registration and the fired
    /// event meet on one key regardless of which spelling each used (`SubagentEnd` is an
    /// alias of `SubagentStop`).
    pub fn canonical(self) -> Self {
        match self {
            Self::SubagentEnd => Self::SubagentStop,
            other => other,
        }
    }

    /// Returns true if this event type uses blocking (deny/allow) semantics.
    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::PreToolUse)
    }

    /// Events that don't support matcher patterns (fire on every occurrence).
    pub fn is_lifecycle(&self) -> bool {
        matches!(
            self,
            Self::SessionStart | Self::SessionEnd | Self::Stop | Self::UserPromptSubmit
        )
    }
}

/// The normalized event envelope sent to hook commands on stdin as JSON.
///
/// Contains common metadata plus an event-specific payload.
/// All field names use camelCase for the JSON wire format.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEventEnvelope {
    pub hook_event_name: HookEventName,
    pub session_id: String,
    pub cwd: String,
    pub workspace_root: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(flatten)]
    pub payload: HookPayload,
}

/// One model's share of a subagent's bill, as the child's own ledger recorded
/// it. Carried by `SubagentStop` (and the `SubagentFinished` session update it
/// is built from), and by `SessionEnd` for the session's own ledger.
///
/// Deliberately not the whole `UsageTotals`: cost is a vendor concept and is
/// always absent in this fork, so putting it on the wire would only invite a
/// reader to divide by zero and believe the answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ModelUsage {
    /// The model that actually ran.
    pub model: String,
    #[serde(rename = "inputTokens", default)]
    pub input_tokens: u64,
    /// Tokens the model generated. Unlike the context-size figure reported as
    /// `tokensUsed`, a rate computed from this one means something.
    #[serde(rename = "outputTokens", default)]
    pub output_tokens: u64,
    #[serde(rename = "modelCalls", default)]
    pub model_calls: u64,
    /// Time inside the API across those calls, which is the honest denominator
    /// for a generation rate. Wall-clock `duration_ms` includes tool execution
    /// and the child's own thinking between calls.
    #[serde(rename = "apiDurationMs", default)]
    pub api_duration_ms: u64,
}

/// The name this struct shipped under in v0.3.6, when `SubagentStop` was the
/// only event carrying it. Kept so existing call sites and any out-of-tree
/// reader keep compiling; the wire format is unchanged either way.
pub type SubagentModelUsage = ModelUsage;

/// Event-specific payload variants, flattened into the envelope JSON via
/// `#[serde(untagged)]`. Grouped to match `HookEventName`.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum HookPayload {
    // ── Session lifecycle ───────────────────────────────────────
    SessionStart {
        source: String,
        #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        #[serde(rename = "agentType", skip_serializing_if = "Option::is_none")]
        agent_type: Option<String>,
    },
    SessionEnd {
        reason: String,
        #[serde(rename = "turnCount", skip_serializing_if = "Option::is_none")]
        turn_count: Option<u64>,
        #[serde(rename = "toolCallCount", skip_serializing_if = "Option::is_none")]
        tool_call_count: Option<u64>,
        /// The session's final CONTEXT size, not what it generated. Same
        /// caveat as `SubagentStop::tokens_used`: dividing it by any duration
        /// yields a throughput-shaped number that measures nothing. Use
        /// `usage_by_model` for a rate.
        #[serde(rename = "tokensUsed", skip_serializing_if = "Option::is_none")]
        tokens_used: Option<u64>,
        /// What ran and what it generated, per model, from the session's own
        /// ledger. Before this existed, only a subagent's spend was visible to
        /// a hook: a top-level `axon --agent X` run reported nothing at all, so
        /// an eval harness could invoke an agent thirty times and leave no
        /// trace. Empty when the session made no model call, or when its ledger
        /// could not be read — `usage_incomplete` distinguishes those.
        #[serde(
            rename = "usageByModel",
            default,
            skip_serializing_if = "Vec::is_empty"
        )]
        usage_by_model: Vec<ModelUsage>,
        /// The session's bill under-counts, or could not be read at all.
        /// Present only when true, so a reader that ignores it cannot silently
        /// treat a partial total as a complete one.
        #[serde(
            rename = "usageIncomplete",
            default,
            skip_serializing_if = "std::ops::Not::not"
        )]
        usage_incomplete: bool,
        /// True when this session is itself a subagent, which also emits
        /// `SubagentStop`. Both events describe the SAME run, so a consumer
        /// that books both double-counts it. There is no way to tell from the
        /// other fields, which is the whole reason this one is here.
        #[serde(
            rename = "isSubagent",
            default,
            skip_serializing_if = "std::ops::Not::not"
        )]
        is_subagent: bool,
    },
    Stop {
        reason: String,
        /// What THIS TURN spent, per model — not the session total.
        ///
        /// `Stop` fires once per turn, so a consumer that sums these across a
        /// session gets the session's spend. `SessionEnd` carries the
        /// cumulative figure instead, and mixing the two scopes is the one
        /// mistake this pair invites: adding a `SessionEnd` total to the
        /// `Stop` totals counts everything twice.
        ///
        /// Deliberately no `tokens_used` here. That field is a context SIZE,
        /// which is a session-scoped quantity; putting it on a per-turn event
        /// beside per-turn counters is how a reader ends up dividing one by the
        /// other. `output_tokens` over `api_duration_ms` is the rate.
        ///
        /// Empty on the teardown `Stop`s that fire when a session closes: no
        /// turn ran at that moment, so there is genuinely nothing to report.
        #[serde(
            rename = "usageByModel",
            default,
            skip_serializing_if = "Vec::is_empty"
        )]
        usage_by_model: Vec<ModelUsage>,
        /// The turn's bill under-counts, or its ledger could not be read.
        /// Present only when true.
        #[serde(
            rename = "usageIncomplete",
            default,
            skip_serializing_if = "std::ops::Not::not"
        )]
        usage_incomplete: bool,
        /// True when the turn belongs to a subagent session.
        ///
        /// A subagent session fires its own `Stop`, and the parent separately
        /// gets a `SubagentStop` reporting the same spend. Observed on a real
        /// spawn: the child's `Stop` and the `SubagentStop` both carried
        /// `outputTokens: 15` for one run. Nothing else in the two payloads
        /// distinguishes the child's `Stop` from the parent's, so a consumer
        /// summing `Stop` alongside `SubagentStop` counts every subagent twice.
        /// Filter on this, or take `Stop` only where it is absent.
        #[serde(
            rename = "isSubagent",
            default,
            skip_serializing_if = "std::ops::Not::not"
        )]
        is_subagent: bool,
    },
    StopFailure {
        error: String,
    },

    // ── Tool events ─────────────────────────────────────────────
    PreToolUse {
        /// The tool the model invoked. For the meta-dispatch tools (`use_tool`
        /// and the external MCP-call tool) this is the resolved underlying tool
        /// (`server__tool`), not the dispatcher — matchers key on it directly.
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
        #[serde(rename = "toolInput")]
        tool_input: serde_json::Value,
        #[serde(rename = "toolInputTruncated")]
        tool_input_truncated: bool,
        #[serde(rename = "permissionMode", skip_serializing_if = "Option::is_none")]
        permission_mode: Option<String>,
        /// The subagent's type when this tool runs inside one (the envelope's `sessionId`
        /// gives its identity); `None` for the top-level session.
        #[serde(rename = "subagentType", skip_serializing_if = "Option::is_none")]
        subagent_type: Option<String>,
    },
    PostToolUse {
        /// Resolved underlying tool for meta-dispatch tools (see `PreToolUse`).
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
        #[serde(rename = "toolInput")]
        tool_input: serde_json::Value,
        #[serde(rename = "toolResult")]
        tool_result: serde_json::Value,
        #[serde(rename = "toolInputTruncated")]
        tool_input_truncated: bool,
        #[serde(rename = "toolResultTruncated")]
        tool_result_truncated: bool,
        #[serde(rename = "durationMs", skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(rename = "isBackgrounded")]
        is_backgrounded: bool,
        #[serde(rename = "subagentType", skip_serializing_if = "Option::is_none")]
        subagent_type: Option<String>,
    },
    PostToolUseFailure {
        /// Resolved underlying tool for meta-dispatch tools (see `PreToolUse`).
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
        #[serde(rename = "toolInput")]
        tool_input: serde_json::Value,
        #[serde(rename = "toolInputTruncated")]
        tool_input_truncated: bool,
        error: String,
        #[serde(rename = "subagentType", skip_serializing_if = "Option::is_none")]
        subagent_type: Option<String>,
    },
    PermissionDenied {
        /// Resolved underlying tool for meta-dispatch tools (see `PreToolUse`).
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
        #[serde(rename = "toolInput")]
        tool_input: serde_json::Value,
        #[serde(rename = "toolInputTruncated")]
        tool_input_truncated: bool,
    },

    // ── User / notification events ──────────────────────────────
    /// Fires when the user submits a prompt.
    UserPromptSubmit {
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
    },
    /// Fires on agent notifications (permission prompts, idle, etc.).
    Notification {
        #[serde(rename = "notificationType")]
        notification_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Compat: some callers use `level` instead of `notificationType`.
        #[serde(skip_serializing_if = "Option::is_none")]
        level: Option<String>,
    },

    // ── Subagent events ─────────────────────────────────────────
    /// Fires when a subagent is spawned.
    SubagentStart {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        #[serde(rename = "subagentType")]
        subagent_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// Fires when a subagent completes.
    SubagentStop {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        #[serde(rename = "subagentType")]
        subagent_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(rename = "exitCode", skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(rename = "durationMs", skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        /// What the subagent actually cost. The session update carries these
        /// already; before they were forwarded here a hook could time a
        /// subagent but never say what it spent, so "which model is worth the
        /// seat" was unanswerable from outside the process.
        #[serde(rename = "tokensUsed", skip_serializing_if = "Option::is_none")]
        tokens_used: Option<u64>,
        #[serde(rename = "toolCalls", skip_serializing_if = "Option::is_none")]
        tool_calls: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        turns: Option<u32>,
        /// What ran, and what it generated, per model. `tokens_used` above is
        /// the child's final CONTEXT size, so dividing it by `duration_ms`
        /// yields a throughput-shaped number that measures nothing. These are
        /// the child's own billing totals: `output_tokens` really is generated
        /// tokens, and `api_duration_ms` is time spent in the API rather than
        /// wall clock, so a rate computed from the two is a real one.
        ///
        /// The model key is authoritative — it is what the child actually
        /// called, not what a config file says it should have called, which is
        /// the only way an outside reader can attribute a run after the config
        /// has moved on. Empty when the child made no model call, or when its
        /// ledger could not be read.
        #[serde(
            rename = "usageByModel",
            default,
            skip_serializing_if = "Vec::is_empty"
        )]
        usage_by_model: Vec<SubagentModelUsage>,
        /// The child's bill under-counts (drain timeout, nested subagent
        /// incomplete, or an apply miss). Present only when true, because a
        /// reader that ignores it should not silently treat a partial total as
        /// a complete one.
        #[serde(
            rename = "usageIncomplete",
            default,
            skip_serializing_if = "std::ops::Not::not"
        )]
        usage_incomplete: bool,
        /// Why it failed. `exit_code` says only that it did.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    // ── Compaction events ───────────────────────────────────────
    PreCompact {
        /// "manual" or "auto".
        source: String,
    },
    PostCompact {
        /// "manual" or "auto".
        source: String,
    },
}

/// Truncate a JSON value if its serialized size exceeds `MAX_PAYLOAD_SIZE`.
///
/// Returns `(possibly_truncated_value, was_truncated)`.
pub fn truncate_payload(value: serde_json::Value) -> (serde_json::Value, bool) {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= MAX_PAYLOAD_SIZE {
        return (value, false);
    }

    // Cut at the largest char boundary <= MAX_PAYLOAD_SIZE so the slice never
    // splits a multibyte codepoint.
    let mut end = MAX_PAYLOAD_SIZE;
    while !serialized.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = serialized[..end].to_string();
    result.push_str(" [truncated]");
    (serde_json::Value::String(result), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The camelCase names a hook script actually reads, and the promise that
    /// an empty ledger adds nothing to the payload.
    #[test]
    fn stop_reports_only_this_turn_not_the_session() {
        let payload = HookPayload::Stop {
            reason: "end_turn".into(),
            usage_by_model: vec![ModelUsage {
                model: "qwen38".into(),
                input_tokens: 12_000,
                output_tokens: 340,
                model_calls: 1,
                api_duration_ms: 17_800,
            }],
            usage_incomplete: false,
            is_subagent: false,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["reason"], "end_turn");
        assert!(
            json.get("isSubagent").is_none(),
            "a parent turn must not carry the subagent marker: {json}"
        );
        assert_eq!(json["usageByModel"][0]["outputTokens"], 340);
        assert_eq!(json["usageByModel"][0]["apiDurationMs"], 17_800);
        // A context size is session-scoped. Putting it on a per-turn event
        // beside per-turn counters is how a reader ends up dividing one by the
        // other, which is the mistake `usageByModel` exists to prevent.
        assert!(
            json.get("tokensUsed").is_none(),
            "Stop must not carry a session-scoped context size: {json}"
        );
    }

    #[test]
    fn teardown_stop_reports_no_turn_usage_and_no_caveat() {
        // The `Stop`s fired while a session closes are not turn ends. Empty
        // AND unflagged is correct there: nothing ran, so nothing is missing.
        let payload = HookPayload::Stop {
            reason: "shutdown".into(),
            usage_by_model: Vec::new(),
            usage_incomplete: false,
            is_subagent: false,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("usageByModel").is_none());
        assert!(json.get("usageIncomplete").is_none());
    }

    #[test]
    fn session_end_usage_by_model_reaches_the_wire() {
        let payload = HookPayload::SessionEnd {
            reason: "shutdown".into(),
            turn_count: Some(12),
            tool_call_count: Some(48),
            tokens_used: Some(187_598),
            usage_by_model: vec![ModelUsage {
                model: "qwen38".into(),
                input_tokens: 180_000,
                output_tokens: 7_598,
                model_calls: 12,
                api_duration_ms: 400_000,
            }],
            usage_incomplete: false,
            is_subagent: false,
        };
        let json = serde_json::to_value(&payload).unwrap();
        let entry = &json["usageByModel"][0];
        assert_eq!(entry["model"], "qwen38");
        assert_eq!(entry["inputTokens"], 180_000);
        // Same reason as the subagent case: `tokensUsed` is a context size, so
        // only `outputTokens` says what the session produced and only
        // `apiDurationMs` is an honest denominator for a rate.
        assert_eq!(entry["outputTokens"], 7_598);
        assert_eq!(entry["apiDurationMs"], 400_000);
        assert_eq!(entry["modelCalls"], 12);
        // These two were declared since before v0.3.6 and passed as None at
        // every dispatch site, so the wire never carried them. Assert they
        // actually serialize now.
        assert_eq!(json["turnCount"], 12);
        assert_eq!(json["toolCallCount"], 48);
        assert!(
            json.get("usageIncomplete").is_none(),
            "a complete bill must not carry the caveat: {json}"
        );
        assert!(
            json.get("isSubagent").is_none(),
            "a top-level session must not carry the subagent marker: {json}"
        );
        // Cost stays off the wire here too.
        assert!(entry.get("costUsdTicks").is_none());
    }

    #[test]
    fn session_end_marks_a_subagent_so_a_consumer_can_avoid_double_counting() {
        // A subagent session emits BOTH SubagentStop and SessionEnd for the
        // same run. Nothing else in the payload distinguishes them, so a
        // consumer that books both counts that run twice.
        let payload = HookPayload::SessionEnd {
            reason: "shutdown".into(),
            turn_count: Some(1),
            tool_call_count: Some(2),
            tokens_used: Some(6_641),
            usage_by_model: Vec::new(),
            usage_incomplete: false,
            is_subagent: true,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["isSubagent"], true);
    }

    #[test]
    fn session_end_reports_an_unreadable_ledger_as_flagged_not_free() {
        // The distinction that matters: empty usage WITH the flag means the
        // session ran and its ledger was lost. Empty usage WITHOUT the flag
        // would claim it genuinely spent nothing.
        let payload = HookPayload::SessionEnd {
            reason: "channel_closed".into(),
            turn_count: None,
            tool_call_count: None,
            tokens_used: None,
            usage_by_model: Vec::new(),
            usage_incomplete: true,
            is_subagent: false,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(
            json.get("usageByModel").is_none(),
            "empty usage is omitted, not sent as []: {json}"
        );
        assert_eq!(
            json["usageIncomplete"], true,
            "an unreadable ledger must be flagged, or it reads as a free session: {json}"
        );
        assert!(json.get("tokensUsed").is_none());
    }

    #[test]
    fn subagent_stop_usage_by_model_reaches_the_wire() {
        let payload = HookPayload::SubagentStop {
            subagent_id: "sa-1".into(),
            subagent_type: "executor".into(),
            description: None,
            exit_code: Some(0),
            duration_ms: Some(41_000),
            tokens_used: Some(31_200),
            tool_calls: Some(6),
            turns: Some(9),
            usage_by_model: vec![SubagentModelUsage {
                model: "laguna".into(),
                input_tokens: 30_000,
                output_tokens: 1_200,
                model_calls: 9,
                api_duration_ms: 18_000,
            }],
            usage_incomplete: false,
            error: None,
        };
        let json = serde_json::to_value(&payload).unwrap();
        let entry = &json["usageByModel"][0];
        assert_eq!(entry["model"], "laguna");
        assert_eq!(entry["inputTokens"], 30_000);
        // The whole reason this field exists: `tokensUsed` is a context size,
        // so only `outputTokens` can answer what the seat produced, and only
        // `apiDurationMs` is an honest denominator for a rate.
        assert_eq!(entry["outputTokens"], 1_200);
        assert_eq!(entry["apiDurationMs"], 18_000);
        assert_eq!(entry["modelCalls"], 9);
        assert!(
            json.get("usageIncomplete").is_none(),
            "a complete bill must not carry the caveat: {json}"
        );
        // Cost is a vendor concept and always absent here. On the wire it would
        // only invite a reader to divide by zero and trust the result.
        assert!(entry.get("costUsdTicks").is_none());
        assert!(entry.get("cost_usd_ticks").is_none());
    }

    #[test]
    fn subagent_stop_omits_usage_when_nothing_ran_but_keeps_the_caveat() {
        let mut payload = HookPayload::SubagentStop {
            subagent_id: "sa-2".into(),
            subagent_type: "scout".into(),
            description: None,
            exit_code: Some(-1),
            duration_ms: Some(2),
            tokens_used: Some(0),
            tool_calls: Some(0),
            turns: Some(0),
            usage_by_model: Vec::new(),
            usage_incomplete: false,
            error: None,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("usageByModel").is_none());
        assert!(json.get("usageIncomplete").is_none());

        // A run whose ledger was lost must say so, or empty usage reads as free.
        if let HookPayload::SubagentStop {
            usage_incomplete, ..
        } = &mut payload
        {
            *usage_incomplete = true;
        }
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["usageIncomplete"], true);
    }

    #[test]
    fn event_name_deser_all_variants() {
        let cases: &[(&str, &str, HookEventName)] = &[
            ("SessionStart", "session_start", HookEventName::SessionStart),
            ("PreToolUse", "pre_tool_use", HookEventName::PreToolUse),
            ("PostToolUse", "post_tool_use", HookEventName::PostToolUse),
            (
                "PostToolUseFailure",
                "post_tool_use_failure",
                HookEventName::PostToolUseFailure,
            ),
            ("SessionEnd", "session_end", HookEventName::SessionEnd),
            ("Stop", "stop", HookEventName::Stop),
            ("StopFailure", "stop_failure", HookEventName::StopFailure),
            ("Notification", "notification", HookEventName::Notification),
            (
                "UserPromptSubmit",
                "user_prompt_submit",
                HookEventName::UserPromptSubmit,
            ),
            (
                "PermissionDenied",
                "permission_denied",
                HookEventName::PermissionDenied,
            ),
            (
                "SubagentStart",
                "subagent_start",
                HookEventName::SubagentStart,
            ),
            ("SubagentStop", "subagent_stop", HookEventName::SubagentStop),
            ("SubagentEnd", "subagent_end", HookEventName::SubagentEnd),
            ("PreCompact", "pre_compact", HookEventName::PreCompact),
            ("PostCompact", "post_compact", HookEventName::PostCompact),
        ];

        for (pascal, snake, expected) in cases {
            let from_pascal: HookEventName =
                serde_json::from_str(&format!("\"{pascal}\"")).unwrap();
            assert_eq!(
                from_pascal, *expected,
                "PascalCase deser failed for {pascal}"
            );

            let from_snake: HookEventName = serde_json::from_str(&format!("\"{snake}\"")).unwrap();
            assert_eq!(from_snake, *expected, "snake_case deser failed for {snake}");
        }
    }

    #[test]
    fn event_name_display_all_variants() {
        let cases: &[(HookEventName, &str)] = &[
            (HookEventName::SessionStart, "session_start"),
            (HookEventName::PreToolUse, "pre_tool_use"),
            (HookEventName::PostToolUse, "post_tool_use"),
            (HookEventName::PostToolUseFailure, "post_tool_use_failure"),
            (HookEventName::SessionEnd, "session_end"),
            (HookEventName::Stop, "stop"),
            (HookEventName::StopFailure, "stop_failure"),
            (HookEventName::Notification, "notification"),
            (HookEventName::UserPromptSubmit, "user_prompt_submit"),
            (HookEventName::PermissionDenied, "permission_denied"),
            (HookEventName::SubagentStart, "subagent_start"),
            (HookEventName::SubagentStop, "subagent_stop"),
            (HookEventName::SubagentEnd, "subagent_stop"), // alias collapses
            (HookEventName::PreCompact, "pre_compact"),
            (HookEventName::PostCompact, "post_compact"),
        ];
        for (event, expected) in cases {
            assert_eq!(&event.to_string(), expected, "Display wrong for {event:?}");
        }
    }

    #[test]
    fn event_name_serde_roundtrip() {
        let name = HookEventName::PreToolUse;
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, "\"pre_tool_use\"");
        let parsed: HookEventName = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, name);
    }

    #[test]
    fn event_name_unknown_rejected() {
        let result = serde_json::from_str::<HookEventName>("\"UnknownEvent\"");
        assert!(result.is_err());
    }

    #[test]
    fn event_name_is_blocking() {
        assert!(HookEventName::PreToolUse.is_blocking());
        for event in [
            HookEventName::SessionStart,
            HookEventName::PostToolUse,
            HookEventName::PostToolUseFailure,
            HookEventName::SessionEnd,
            HookEventName::Stop,
            HookEventName::StopFailure,
            HookEventName::Notification,
            HookEventName::UserPromptSubmit,
            HookEventName::PermissionDenied,
            HookEventName::SubagentStart,
            HookEventName::SubagentStop,
            HookEventName::SubagentEnd,
            HookEventName::PreCompact,
            HookEventName::PostCompact,
        ] {
            assert!(!event.is_blocking(), "{event:?} should not be blocking");
        }
    }

    #[test]
    fn event_name_is_lifecycle() {
        let lifecycle = [
            HookEventName::SessionStart,
            HookEventName::SessionEnd,
            HookEventName::Stop,
            HookEventName::UserPromptSubmit,
        ];
        for event in lifecycle {
            assert!(event.is_lifecycle(), "{event:?} should be lifecycle");
        }

        let matchable = [
            HookEventName::PreToolUse,
            HookEventName::PostToolUse,
            HookEventName::PostToolUseFailure,
            HookEventName::PermissionDenied,
            HookEventName::StopFailure,
            HookEventName::Notification,
            HookEventName::SubagentStart,
            HookEventName::SubagentStop,
            HookEventName::SubagentEnd,
            HookEventName::PreCompact,
            HookEventName::PostCompact,
        ];
        for event in matchable {
            assert!(
                !event.is_lifecycle(),
                "{event:?} should support matchers, not be lifecycle"
            );
        }
    }

    #[test]
    fn truncate_small_payload() {
        let value = serde_json::json!({"key": "small"});
        let (result, truncated) = truncate_payload(value.clone());
        assert!(!truncated);
        assert_eq!(result, value);
    }

    #[test]
    fn truncate_large_payload() {
        let big_string = "x".repeat(MAX_PAYLOAD_SIZE + 1000);
        let value = serde_json::Value::String(big_string);
        let (result, truncated) = truncate_payload(value);
        assert!(truncated);
        let s = result.as_str().unwrap();
        assert!(s.ends_with("[truncated]"));
        // Serialized size of the result string value should be <= MAX_PAYLOAD_SIZE + overhead
        assert!(s.len() < MAX_PAYLOAD_SIZE + 100);
    }

    #[test]
    fn truncate_large_payload_cuts_on_char_boundary() {
        // '€' is 3 bytes, so the MAX_PAYLOAD_SIZE-th byte lands mid-codepoint.
        let value = serde_json::Value::String("€".repeat(MAX_PAYLOAD_SIZE));
        let (result, truncated) = truncate_payload(value);
        assert!(truncated);
        assert!(result.as_str().unwrap().ends_with("[truncated]"));
    }

    #[test]
    fn envelope_serializes_camel_case() {
        let envelope = HookEventEnvelope {
            hook_event_name: HookEventName::SessionStart,
            session_id: "test-session".into(),
            cwd: "/tmp".into(),
            workspace_root: "/tmp".into(),
            timestamp: "2025-01-01T00:00:00Z".into(),
            transcript_path: None,
            client_identifier: None,
            prompt_id: None,
            payload: HookPayload::SessionStart {
                source: "new".into(),
                model_id: Some("axon-3".into()),
                agent_type: None,
            },
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains("hookEventName"));
        assert!(json.contains("sessionId"));
        assert!(json.contains("workspaceRoot"));
        assert!(json.contains("modelId"));
        // Should NOT contain snake_case versions
        assert!(!json.contains("hook_event_name"));
        assert!(!json.contains("session_id"));
    }
}
