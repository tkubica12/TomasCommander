# UI lab / round 01

One comparison page, five switchable designs, shared mock files and commands.
This is the explicitly separate design exercise, not the real product.

The product is Rust-first and must be lightweight with measured performance.
This dependency-free browser lab is disposable presentation/mock scaffolding,
not a choice of production renderer or authorization to implement production
services in JavaScript. Its browser memory/startup cannot establish the native
product's performance. The owner approved browser previews for this design round
only; the selected real product will be a standalone Rust-first Windows app.

## Run

From `D:\TomasCommander`, using Node.js 22 or newer:

```powershell
node prototypes\ui\server.mjs
```

Open http://127.0.0.1:4177 in a browser. The server listens on loopback only,
serves an explicit asset allowlist, and has no write API. Ctrl+C stops it.
Set `TC_UI_PORT` if that port is occupied; do not terminate another process.

## Compare and give feedback

Switch among Commander, Ledger, Studio, Console, and Quiet using the top tabs.
State stays shared when switching; **Reset demo** restores the original files.
Try dark/light mode and resizing the divider with mouse or keyboard.
Use **Accent** to cycle blue, red-orange, green, and yellow. Switching themes
keeps the chosen accent family and uses its canonical light/dark shade.

Use **Design notes** to record what to keep/change for each design and mark a
preferred candidate. Notes are stored only in browser local storage. They are
not automatically delivered to the agent. Export feedback JSON and attach it
in chat, or simply describe feedback in chat using the design names.

## Suggested comparison tasks

1. Focus a file list. Navigate with arrows, select with Space or Shift+arrows,
   switch panes with Tab, and open/return with Enter/Backspace.
2. Filter a folder, change sorting, jump via Places, and pin a new location.
3. Preview and cancel copy/move/delete. Approve a mock operation and observe
   the changed lists; reset and try a conflict or Restricted destination.
4. Open Ctrl+Shift+P. Choose a command, or type
   `Copy my Penta hackathon presentation to Kosik`.
5. Search for `Penta`, or select the labeled Serbian holiday AI example.
6. Explore the labeled answer, voice, and external-app previews.

Ctrl+Shift+C copies, Ctrl+Shift+M moves, Delete deletes, Ctrl+Shift+S focuses
sorting, Ctrl+Shift+F searches, and Ctrl+F focuses the active-pane filter.
Ctrl+Shift+Left/Right resizes panes when file-pane controls are focused.
Divider Left/Right also works; Home balances it. Text inputs retain their
editing shortcuts. Escape closes dialogs or clears file selections.

All operations modify in-memory synthetic data only. AI examples are fixed,
labeled demonstrations. No real files, model/MCP connections, microphone, or
external applications are accessed. There are no runtime packages or network
assets. Future real features must satisfy the product Goal Cards.

## Architecture

`model.mjs`: fixtures, shared state, command definitions, navigation, plans,
operations, and search. `app.mjs`: controls, rendering, focus, and dialog flows.
`styles.css`: five presentation variants over the same DOM and state.
`server.mjs`: dependency-free local asset server.

Canonical grayscale and blue accent values come from the html-docs design
tokens. Layout CSS is specific to this application exploration, not a document
template.

## Checks

```powershell
node --test prototypes\ui\model.test.mjs
$env:TC_UI_ARTIFACTS = '<absolute session evidence directory>'
node prototypes\ui\browser.test.mjs
```

Browser checks use the installed Edge in headless mode via its debugging
protocol, without adding packages. The server must already be running.
`TC_UI_BROWSER` can override the browser executable; `TC_UI_URL` can override
the loopback URL. The runner closes its own browser and removes only its named
temporary profile. Evidence includes interaction results and screenshots for
all five designs in both themes at laptop and desktop sizes.

Automated browser key events do not prove physical CZ/US keyboard behavior.
That Goal Card check remains pending actual-layout testing, as does owner
acceptance of a design.
