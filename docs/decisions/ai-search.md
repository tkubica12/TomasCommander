# Experimental AI filename discovery

AI search interprets one explicitly confirmed Czech/English prompt through the
configured Luna route, then ranks actual files locally in the active folder and
descendants. Only the prompt/fixed instructions are sent, not candidates,
paths or file contents. It does not recognize image contents.
Search/Enter submits the prompt directly; there is no separate consent checkbox.
AI search lives in the multiline command palette through `/ai-search` or a
free-text prompt with no command matches, not a separate toolbar entry.
The palette shows a prompt, Search and Close, then progress/results or an error.
Connection settings are offered on errors; matching reasons remain in result
tooltips rather than cluttering rows.

This is a single intent interpretation followed by deterministic local discovery,
not a tool-calling agent. It cannot inspect directory structure iteratively,
run glob/grep, search outside the chosen root or send the current path as model
context. A broader read-only harness requires a separately defined access scope
and updated data/cost boundaries; UI simplification does not enable it.

Typed bounded output expresses concepts/folders/types. Unknown fields, malformed,
ambiguous/unsupported output and provider errors fail explicitly; no unrequested
model fallback or extra request is made. Local discovery skips links/junctions
and reports cancellation/errors/truncation. Results explain filename/folder/type
matches and revalidate activation.

Ctrl+Shift+F remains deterministic local filename search. An AI prompt never
turns into shell instructions or filesystem approval. Pilot evidence applies to
small synthetic workloads, not arbitrary personal-data relevance.
See [runtime data/cost boundaries](ai-runtime.md).
