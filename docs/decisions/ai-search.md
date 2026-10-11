# Experimental AI filename discovery

AI search interprets one explicitly confirmed Czech/English prompt through the
configured Luna route, then ranks actual files locally in the active folder and
descendants. Only the prompt/fixed instructions are sent, not candidates,
paths or file contents. It does not recognize image contents.

Typed bounded output expresses concepts/folders/types. Unknown fields, malformed,
ambiguous/unsupported output and provider errors fail explicitly; no unrequested
model fallback or extra request is made. Local discovery skips links/junctions
and reports cancellation/errors/truncation. Results explain filename/folder/type
matches and revalidate activation.

Ctrl+Shift+F remains deterministic local filename search. An AI prompt never
turns into shell instructions or filesystem approval. Pilot evidence applies to
small synthetic workloads, not arbitrary personal-data relevance.
See [runtime data/cost boundaries](ai-runtime.md).
