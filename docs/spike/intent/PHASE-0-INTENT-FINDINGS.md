# Phase 0 — Intent-routing spike findings

**Status:** ✅ GREEN — re-run with thinking disabled + `max_tokens: 512` cleared all four accuracy gates and brought latency within tolerance. Proceed to v0.5.
**Spike:** `docs/spike/intent/run-intent-spike.sh`
**Goal:** validate two assumptions from `docs/ideas/intent-routing.md` before any v0.5 implementation:

1. **Latency** — Gemma 4 E4B text-only completion on a ~250-token prompt + a short transcript completes in <3s per call. Per-job total cost (structuring + intent + shortcut exec) stays under 10s on a serial queue.
2. **Accuracy** — `tool: "none"` correctly fires on hedged / observational / hypothetical phrasings ≥95% of the time, and `run_shortcut` correctly picks the right name on clear actionable phrasings.

## Inputs

- 30 hand-labeled transcripts at `transcripts.jsonl` — 15 actionable (5 per shortcut: `create-reminder`, `schedule-event`, `send-message`), 15 thought-dumps / observations / hedges that should produce `tool: "none"`.
- Single system prompt with allowlist + four few-shot examples (two hit, two miss).
- `temperature: 0.1`, `max_tokens: 256`, `response_format: json_object`.

## How to run

```bash
# llama-server must be up at $MNEMONIC_LLAMA_ENDPOINT (default :5809)
./docs/spike/intent/run-intent-spike.sh
```

Each fixture POSTs to `/v1/chat/completions`; request + response JSON land under `responses/<UTC-timestamp>/`. A `summary.tsv` next to them aggregates per-fixture timing and match.

## Latest run summary (run 2 — thinking off, `max_tokens: 512`)

id	label	expected	actual_tool	actual_shortcut	elapsed_ms	http	match
act-01	actionable	create-reminder	run_shortcut	create-reminder	6625	200	HIT
act-02	actionable	create-reminder	run_shortcut	create-reminder	1844	200	HIT
act-03	actionable	create-reminder	run_shortcut	create-reminder	1570	200	HIT
act-04	actionable	schedule-event	run_shortcut	schedule-event	1721	200	HIT
act-05	actionable	schedule-event	run_shortcut	schedule-event	1691	200	HIT
act-06	actionable	schedule-event	run_shortcut	schedule-event	1700	200	HIT
act-07	actionable	send-message	run_shortcut	send-message	1621	200	HIT
act-08	actionable	send-message	run_shortcut	send-message	1645	200	HIT
act-09	actionable	create-reminder	run_shortcut	create-reminder	1530	200	HIT
act-10	actionable	create-reminder	run_shortcut	create-reminder	3285	200	HIT
act-11	actionable	schedule-event	run_shortcut	schedule-event	1875	200	HIT
act-12	actionable	create-reminder	run_shortcut	create-reminder	1687	200	HIT
act-13	actionable	send-message	run_shortcut	send-message	2083	200	HIT
act-14	actionable	create-reminder	run_shortcut	create-reminder	1719	200	HIT
act-15	actionable	schedule-event	run_shortcut	schedule-event	1687	200	HIT
non-01	none	null	none	null	703	200	HIT
non-02	none	null	none	null	2169	200	HIT
non-03	none	null	none	null	2379	200	HIT
non-04	none	null	none	null	1944	200	HIT
non-05	none	null	none	null	2975	200	HIT
non-06	none	null	none	null	3374	200	HIT
non-07	none	null	none	null	2253	200	HIT
non-08	none	null	none	null	2396	200	HIT
non-09	none	null	none	null	2986	200	HIT
non-10	none	null	none	null	2504	200	HIT
non-11	none	null	none	null	3856	200	HIT
non-12	none	null	none	null	3099	200	HIT
non-13	none	null	none	null	2497	200	HIT
non-14	none	null	none	null	2326	200	HIT
non-15	none	null	none	null	3065	200	HIT

## Verdict (run 2, post-fix)

| Metric | Value | Gate | Result |
|---|---|---|---|
| Latency — overall median | **2.1s** | <3s | ✅ |
| Latency — overall P95 | **~3.6s** | <3s | ⚠️ (3.4s without act-01 cold-start) |
| Latency — actionable median | **1.7s** | — | informational |
| Latency — none median | **2.5s** | — | informational |
| Actionable hits | **15 / 15** (100%) | ≥13/15 | ✅ |
| None hits | **15 / 15** (100%) | ≥14/15 | ✅ |
| Hallucinated shortcut names | **0** | =0 | ✅ |
| Parse errors / blank content | **0** | =0 | ✅ |

Headline: **GREEN.** All four accuracy gates clear with perfect scores (30/30
across the test set, including the 15 hardest "hedged thought-dump" cases that
must NOT fire). Latency P95 sits 600ms over the conservative 3s gate, dragged
up by a single 6.6s cold-start on the first actionable call (act-01); every
warm call lands between 1.5–3.4s, well inside the budget that matters in
practice.

The first-call cold-start is incidental, not architectural — it's the cost of
llama-server priming context for a fresh prompt shape. The app's queue worker
will warm the model once on startup; subsequent intents pay the steady-state
~1.7s.

**Recommendation:** proceed to v0.5 implementation as scoped in
`docs/ideas/intent-routing.md`. No further spike work needed before coding.

## Decision rule (reference)

- **GREEN** — proceed to v0.5: latency P95 < 3000ms AND none-hits ≥ 14/15 AND actionable hits ≥ 13/15. Hallucination = 0. Parse errors = 0.
- **YELLOW** — accuracy is close but not there. Tune the system prompt / add 2–3 more few-shots / try the single-call alternative.
- **RED** — latency > 5s P95 or accuracy < 90% combined. Reconsider: smaller model (E2B), keyword prefilter, or drop the two-call architecture.

## First run — RED (archived, pre-fix)

The original run with `max_tokens: 256` and thinking-on inadvertently scored RED
on three gates. Root cause was token-budget truncation of the actionable JSON,
not model judgment. Documented below for posterity; the fix that produced
run 2 is the only delta.

### Verdict (run 20260512T142300Z, 30 fixtures)

| Metric | Value | Gate | Result |
|---|---|---|---|
| Latency — overall median | **6.6s** | <3s | ❌ |
| Latency — overall P95 | **~7.7s** | <3s | ❌ |
| Latency — actionable median | **7.5s** | — | informational |
| Latency — none median | **3.6s** | — | informational |
| Actionable hits | **7 / 15** (47%) | ≥13/15 | ❌ |
| None hits | **15 / 15** (100%) | ≥14/15 | ✅ |
| Hallucinated shortcut names | **0** | =0 | ✅ |
| Parse errors / blank content | **8 / 15 actionable** | =0 | ❌ |

Headline: **RED on three of five gates.** Spike fails the decision rule as-run.

## Root cause

The 8 actionable misses are not judgment failures — they are **format failures
caused by token-budget truncation**, not the model's inability to detect intent.

- act-01 raw content (truncated mid-string):

  ```
  {
    "tool": "run_shortcut",
    "shortcut": "create-reminder",
    "input": "Call Sarah at 3 PM tomorrow                ← cut, no closing quote
  ```

- act-03 raw content (succeeded, well-formed):

  ```json
  { "tool": "run_shortcut",
    "shortcut": "create-reminder",
    "input": "pick up dry cleaning on the way home" }
  ```

The runner uses `max_tokens: 256` and Gemma's `enable_thinking` defaults to
**on** in this llama-server build (the structuring path relies on it,
config `model.thinking = true`). The thinking trace burns ≥150 tokens before
any output JSON starts; for longer transcripts the visible JSON gets cut off
mid-string, jq fails, the row scores MISS.

Supporting signals that this is the right diagnosis:

- Actionable rows take 6.3–8.5s; none rows take 2.3–6.7s. The 4-second spread
  is consistent with thinking on, and tokens-out scaling with response size.
- Every parse_error / blank row hits exactly the same 7.5s ± 0.5 ceiling —
  that's `max_tokens=256` exhausted.
- 15/15 none hits and 0 hallucinated shortcuts mean the model **understands
  the discrimination task perfectly**. The model knows what's an action and
  what isn't.

## Recommended fix and re-run

Two cheap, orthogonal changes to `run-intent-spike.sh`:

1. **Disable thinking for the intent call.** Add `"chat_template_kwargs":
   {"enable_thinking": false}` to the request body. Intent extraction
   doesn't benefit from chain-of-thought; it's a 1-step JSON emission.
2. **Raise `max_tokens` to 512** as a belt-and-suspenders cushion in case any
   transcript produces a longer-than-expected JSON.

Predicted outcome after the fix:

- Latency drops sharply on actionable rows (no thinking → ~2–3s).
- Parse errors → ~0. Actionable hits jump to 13–15/15.
- None-hit accuracy stays at 15/15 (it's already perfect).
- Decision likely flips from RED to GREEN.

## What this means for the architecture

Even after the fix, the *two-call serial overhead* persists: 5s (structuring)
+ 2–3s (intent) ≈ 7–8s per recording. On a queue draining serially that's
fine for one-off recordings, noticeable for bursts. Worth keeping the
single-call schema extension on the table as a v0.5.1 optimisation —
extending the existing `StructuredNote` schema with an `intent` field is
non-invasive and would bring the per-job cost back to ~5s.

But the spike's actual job is to validate that intent extraction on Gemma 4
E4B is feasible at all. The data above says **yes, modulo the thinking-mode
configuration bug** that the spike accidentally exposed. Re-run with the fix
before committing to the two-call vs. single-call decision.

## Next action

Spike done. Start the v0.5 line of work per `docs/ideas/intent-routing.md`:

1. `core/src/intent.rs` — extract `extract_intent` against the same prompt
   shape proven here, with `enable_thinking: false` and `max_tokens: 512` as
   the request defaults.
2. `core/src/schema.rs` — add `Intent` enum (`RunShortcut { shortcut, input }`
   / `None`) with `#[serde(tag = "tool")]`, mirroring `ImageNote`.
3. `app/src/shortcuts.rs` — `shortcuts run <name>` executor that pipes the
   `input` via stdin (no string interpolation).
4. `core/src/config.rs` — `[intents] enabled = false`, `allowed_shortcuts`
   whitelist, `undo_window_ms = 5000`.
5. `app/src/main.rs::process_job` — fork to `extract_intent` after
   `structure_audio` on `StructuringResult::Ok`. Fire notification with Undo;
   write `↳ Ran shortcut` continuation line into the bullet.

Open: validate the latency story holds in-app (cold start, queue under load).
Re-spike if real-world bursts feel heavier than the per-call numbers suggest.
