# Connected answers: experimental, live acceptance incomplete

The Rust app-side WebIQ client/credential/source boundary is implemented but
has not completed authorized live source-and-answer acceptance. Offline parser
tests and synthetic UI observations do not prove a working integration.
WorkIQ mode is a typed permission/configuration proposal, not an authenticated
workplace connector. Mail, Teams, SharePoint and Calendar access are not claimed.

## Public source route

The fixed endpoint is `https://api.microsoft.ai/v3/mcp`. Only an explicitly entered
public question and fixed parameters are sent; no local files/paths/history or
workplace passages. Source-mode/config/question changes invalidate consent and
clear pending evidence. Never paste private data into a public question.

The MCP client pins protocol 2025-06-18, initializes, checks tools and performs
one reviewed `web` read. Unknown/changed/paginated schemas and unsolicited server
requests fail. No model-selected tools, arbitrary URL fetching, roots, sampling,
elicitation, automatic retries or workplace-to-public fallback.

At most three passages are retained with original URLs, titles, exact text and
provenance. They are untrusted source excerpts, not generated answers. The narrow
answer parser permits up to four exact quoted claims tied to actual passage IDs
and explicit missing/conflicting evidence. Textual quotation does not establish
truth, relevance or completeness by itself.

Source URLs are rechecked before explicit Windows Shell browser handoff. Only
ordinary public ASCII HTTP/HTTPS without credential/private-host hazards passes.
Browser redirects/network behavior after handoff are outside the app boundary.
No successful live synthesis or complete native keyboard/browser-URL acceptance
is claimed.

## Local key

Enter only your own existing key in the masked local field and explicitly choose
Save. This stores/replaces exactly
`TomasCommander:WebIQ:https://api.microsoft.ai/v3/mcp`
in your Windows Credential Manager, not JSON, arguments or environment.
The app never enumerates the vault or borrows another app's credentials.
Disconnect/close clears pending input and cancels work.

OS-protected user storage is not isolation from other same-user processes.
Owned buffers are cleared normally, but renderer/parser/OS/crash buffers are not
guaranteed erased. Do not put a key in chat or source control.

## Bounds

Question <=1,000 UTF-8 bytes; one concurrent read; <=8 MCP POSTs/job and 16
durable lifetime source POST attempts. Each of three passages <=1,200 bytes;
response <=65,536 bytes and <=16 SSE events. Parent deadline 30 seconds,
network stage limits 5 seconds. Unknown transport outcomes block rather than retry.

`answer-scenarios.json` binds the two-question public pilot. Its exact existing
grant admits those questions/public passages to the configured Sol route only,
preserving history nine and capping cumulative inference at eleven. It never
enables generic stronger-model or workplace inference. Requires actual local
credential/configuration/consent and a real source receipt, not cached mock data.
Budgets/leases live beside user-local AI configuration and are never reset
automatically. Cancellation may still incur provider charges.
