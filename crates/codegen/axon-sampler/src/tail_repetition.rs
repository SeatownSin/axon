//! Client-side tail-repetition detection.
//!
//! The server-side doom-loop check ([`crate::doom_loop`]) only exists on the
//! vendor's streaming `/v1/responses` contract, so a local model degenerating
//! on Chat Completions or Anthropic Messages goes completely unnoticed. This
//! module reproduces the cheapest and most reliable of the server's trigger
//! classes -- `tail_repetition` -- from the text the client is already
//! accumulating, and emits the same [`DoomLoopSignal`] shape.
//!
//! **Report-only by design.** Detections are logged and attached to the
//! response; they are deliberately NOT fed to the abort path. A false positive
//! here would kill a good response and burn retry budget, and a real detection
//! can mask the underlying cause: the 863-repeat `</think>` storm that
//! motivated this was a *parser* misconfiguration, not model degeneration, and
//! an automatic retry would have hidden it. Arm the abort only once the
//! false-positive rate on real workloads is known.
//!
//! Detection is exact byte periodicity of the tail. That choice is what keeps
//! false positives away: legitimately repetitive output -- markdown tables,
//! lists, arrays of similar JSON objects -- varies between rows and is
//! therefore not byte-periodic. Only genuinely stuck generation repeats the
//! same bytes verbatim, dozens of times, without drift.

use axon_sampling_types::doom_loop::DoomLoopSignal;

/// Longest repeating unit considered. A stuck model repeats a token or a
/// short phrase; beyond this length repetition is structure, not degeneration.
const MAX_PERIOD: usize = 128;

/// Only the tail of the accumulated text is examined, so the cost per scan is
/// independent of how long the response has grown.
const WINDOW: usize = 8192;

/// The repeated region must cover at least this many bytes.
const MIN_SPAN: usize = 256;

/// ...and the unit must repeat at least this many times.
const MIN_REPEATS: u32 = 8;

/// A unit built from one or two distinct bytes (`-`, `\n`, `ab`) is the
/// ambiguous case: horizontal rules and blank-line runs are legitimate output,
/// so these need a longer run before they count.
///
/// This keys on the unit's *content*, not its length, because a length-based
/// guard is trivially defeated: a run of 300 dashes fails the bar at period 1
/// but matches again at period 4 as `----`, with the same bytes and the same
/// span. Byte diversity is invariant across those descriptions.
const LOW_DIVERSITY_BYTES: usize = 2;
const LOW_DIVERSITY_MIN_SPAN: usize = 512;

/// Rescan only after this many new bytes, so a stream of one-token deltas does
/// not trigger a scan per token.
const SCAN_STRIDE: usize = 64;

/// Channel labels, matching the server's vocabulary so the emitted trigger
/// labels are indistinguishable from server-reported ones.
pub(crate) const CHANNEL_RESPONSE: &str = "response";
pub(crate) const CHANNEL_THINKING: &str = "thinking";

/// A repeating tail: `unit` repeated `repeats` times covering `span` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Detection {
    pub(crate) period: usize,
    pub(crate) repeats: u32,
    pub(crate) span: usize,
}

/// Find the shortest repeating unit at the very end of `text`, if the run is
/// long enough to be degenerate rather than incidental.
///
/// Returns the *shortest* qualifying period, which is the true unit: text
/// repeating with period 8 also repeats with period 16, and reporting 16 would
/// halve the apparent repeat count.
pub(crate) fn detect(text: &str) -> Option<Detection> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    if n < MIN_SPAN {
        return None;
    }
    let window = &bytes[n - n.min(WINDOW)..];
    let wn = window.len();

    for period in 1..=MAX_PERIOD.min(wn / 2) {
        // Cheap rejection first: unless the final block already equals the one
        // before it, this period cannot be the unit. This costs one block
        // compare for the overwhelmingly common non-repeating case.
        let last = &window[wn - period..];
        if &window[wn - 2 * period..wn - period] != last {
            continue;
        }

        let mut repeats: u32 = 2;
        let mut start = wn - 2 * period;
        while start >= period && &window[start - period..start] == last {
            start -= period;
            repeats += 1;
        }

        let span = wn - start;
        // The span is the unit repeated, so its byte diversity equals the
        // unit's -- measure the unit, which is at most MAX_PERIOD bytes.
        let min_span = if distinct_bytes(last) <= LOW_DIVERSITY_BYTES {
            LOW_DIVERSITY_MIN_SPAN
        } else {
            MIN_SPAN
        };
        if repeats >= MIN_REPEATS && span >= min_span {
            return Some(Detection {
                period,
                repeats,
                span,
            });
        }
    }
    None
}

/// Count distinct byte values in `unit` (at most [`MAX_PERIOD`] bytes).
fn distinct_bytes(unit: &[u8]) -> usize {
    let mut seen = [0u64; 4];
    let mut count = 0;
    for &b in unit {
        let (word, bit) = (usize::from(b) / 64, 1u64 << (u32::from(b) % 64));
        if seen[word] & bit == 0 {
            seen[word] |= bit;
            count += 1;
        }
    }
    count
}

/// Streaming wrapper: fed the accumulated channel text as it grows, reports at
/// most once per channel per response.
#[derive(Debug, Default)]
pub(crate) struct TailRepetitionWatch {
    scanned_len: usize,
    reported: bool,
}

impl TailRepetitionWatch {
    /// Observe the accumulated text for one channel. Returns a signal the
    /// first time a degenerate tail is seen, then stays quiet.
    ///
    /// `acc` is the caller's existing accumulator, so this adds no buffering.
    pub(crate) fn observe(&mut self, acc: &str, channel: &str) -> Option<DoomLoopSignal> {
        if self.reported || acc.len() < self.scanned_len + SCAN_STRIDE {
            return None;
        }
        self.scanned_len = acc.len();
        let found = detect(acc)?;
        self.reported = true;
        // Reuse the canonical grammar parser rather than re-encoding the label
        // format here, so a local signal is byte-identical to a server one.
        Some(DoomLoopSignal::parse(&format!(
            "tail_repetition:{}@{}",
            found.repeats, channel
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_sampling_types::doom_loop::DoomLoopSignalKind;

    /// The failure that motivated this: a reasoning parser left inert, so the
    /// model's close tag leaked into content and repeated 863 times.
    #[test]
    fn detects_the_think_tag_storm() {
        let text = "</think>".repeat(863);
        let found = detect(&text).expect("storm must be detected");
        assert_eq!(found.period, 8);
        assert!(found.repeats >= 100, "repeats: {}", found.repeats);
    }

    #[test]
    fn detects_single_character_degeneration() {
        let text = format!("Here is the answer: {}", "a".repeat(1200));
        let found = detect(&text).expect("runaway single char must be detected");
        assert_eq!(found.period, 1);
    }

    #[test]
    fn reports_the_shortest_unit_not_a_multiple() {
        let text = "ab".repeat(600);
        let found = detect(&text).expect("detected");
        assert_eq!(found.period, 2, "must report the true unit, not 4 or 6");
    }

    // ---- false-positive shapes: legitimate output that must NOT trip ----

    #[test]
    fn markdown_table_is_not_a_loop() {
        let mut t = String::from("| name | value | note |\n|---|---|---|\n");
        for i in 0..200 {
            t.push_str(&format!("| row_{i} | {i} | note number {i} |\n"));
        }
        assert_eq!(detect(&t), None, "table rows differ, so not byte-periodic");
    }

    #[test]
    fn json_array_of_similar_objects_is_not_a_loop() {
        let mut t = String::from("[\n");
        for i in 0..300 {
            t.push_str(&format!("  {{\"id\": {i}, \"ok\": true}},\n"));
        }
        assert_eq!(detect(&t), None);
    }

    #[test]
    fn horizontal_rule_is_not_a_loop() {
        let t = format!("Section\n{}\nNext section\n", "-".repeat(300));
        assert_eq!(
            detect(&t),
            None,
            "a long rule is ASCII art, and it is not at the tail anyway"
        );
    }

    /// Regression: a length-based guard rejected this at period 1 and then
    /// accepted the very same bytes at period 4 as `----`. The bar must key on
    /// the unit's byte diversity so every description of a run agrees.
    #[test]
    fn a_rule_at_the_tail_is_rejected_at_every_period() {
        let t = format!("Section\n{}", "-".repeat(300));
        assert_eq!(detect(&t), None);
        // 4 and 6 are the periods that previously slipped through.
        for pad in [0usize, 1, 2, 3] {
            let t = format!("Section\n{}", "-".repeat(300 + pad));
            assert_eq!(detect(&t), None, "pad {pad} must not qualify");
        }
    }

    #[test]
    fn blank_line_runs_are_low_diversity_and_need_a_long_run() {
        let t = format!("Done.{}", "\n".repeat(300));
        assert_eq!(detect(&t), None);
    }

    #[test]
    fn indented_code_block_is_not_a_loop() {
        let mut t = String::new();
        for i in 0..120 {
            t.push_str(&format!("    let value_{i} = compute({i});\n"));
        }
        assert_eq!(detect(&t), None);
    }

    #[test]
    fn ordinary_prose_is_not_a_loop() {
        let t = "The quick brown fox jumps over the lazy dog. ".repeat(3);
        assert_eq!(detect(&t), None, "too few repeats and too short a span");
    }

    #[test]
    fn short_input_is_never_a_loop() {
        assert_eq!(detect(""), None);
        assert_eq!(detect("hello"), None);
        assert_eq!(
            detect(&"ab".repeat(20)),
            None,
            "span 40 is far under the bar"
        );
    }

    #[test]
    fn multibyte_text_does_not_panic_and_still_detects() {
        // Byte-level periodicity holds for repeated multi-byte sequences.
        let t = "→→".repeat(400);
        let found = detect(&t).expect("detected");
        assert!(found.span >= MIN_SPAN);
    }

    // ---- streaming wrapper ----

    #[test]
    fn watch_reports_once_and_then_stays_quiet() {
        let mut watch = TailRepetitionWatch::default();
        let acc = "</think>".repeat(400);
        let first = watch.observe(&acc, CHANNEL_RESPONSE);
        assert!(first.is_some(), "first crossing must report");
        let longer = "</think>".repeat(800);
        assert!(
            watch.observe(&longer, CHANNEL_RESPONSE).is_none(),
            "must not spam once reported"
        );
    }

    #[test]
    fn watch_emits_a_label_matching_the_server_grammar() {
        let mut watch = TailRepetitionWatch::default();
        let acc = "</think>".repeat(400);
        let signal = watch.observe(&acc, CHANNEL_THINKING).expect("signal");
        assert!(matches!(signal.kind, DoomLoopSignalKind::TailRepetition(_)));
        assert_eq!(signal.channel, CHANNEL_THINKING);
        assert!(
            signal.raw.starts_with("tail_repetition:"),
            "raw: {}",
            signal.raw
        );
    }

    /// Measures the cost of the watch over a realistic response. Printed with
    /// `--nocapture`; the assertion is a loose regression guard, not a
    /// benchmark -- it exists so an accidentally quadratic rewrite is caught.
    #[test]
    fn watch_cost_over_a_full_response_is_negligible() {
        let start = std::time::Instant::now();
        let mut watch = TailRepetitionWatch::default();
        let mut acc = String::new();
        // ~4000 deltas of a few bytes each: a long local-model response.
        for i in 0..4000 {
            acc.push_str("word ");
            acc.push_str(&i.to_string());
            acc.push(' ');
            let _ = watch.observe(&acc, CHANNEL_RESPONSE);
        }
        let elapsed = start.elapsed();
        println!(
            "tail-repetition watch: {} deltas, {} bytes accumulated, total {:?} ({:?}/delta)",
            4000,
            acc.len(),
            elapsed,
            elapsed / 4000
        );
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "watch cost {elapsed:?} over a whole response -- suspect an algorithmic regression"
        );
    }

    #[test]
    fn watch_stays_quiet_on_healthy_output() {
        let mut watch = TailRepetitionWatch::default();
        let mut acc = String::new();
        for i in 0..400 {
            acc.push_str(&format!("token {i} "));
            assert!(watch.observe(&acc, CHANNEL_RESPONSE).is_none());
        }
    }
}
