export const ROOT = "C:\\Demo";
export const DESIGNS = [
  { id:"commander", name:"Commander", category:"DENSE / 01", description:"Double-rule panes, compact rows, and an always-visible action strip." },
  { id:"ledger", name:"Ledger", category:"DENSE / 02", description:"A location rail and ledger-like rows. More structure, fewer overlays." },
  { id:"studio", name:"Studio", category:"TERMINAL / 03", description:"Two generous file cards and a command dock. Space to think." },
  { id:"console", name:"Console", category:"TERMINAL / 04", description:"A working console with contextual detail and an operation trail." },
  { id:"minimal", name:"Quiet", category:"MINIMAL / 05", description:"A typographic file workspace. Only what you need, when you need it." },
];
export const COMMANDS = [
  { id:"copy", name:"Copy to other pane", hint:"Ctrl Shift C", detail:"Preview a copy of selected files or the focused item." },
  { id:"move", name:"Move to other pane", hint:"Ctrl Shift M", detail:"Approval required. Mock data only." },
  { id:"delete", name:"Delete selected items", hint:"Delete", detail:"Approval required. Mock data only." },
  { id:"search", name:"Search mock files", hint:"Ctrl Shift F", detail:"Find by name, or try the labeled AI demo." },
  { id:"favorites", name:"Favorite locations", hint:"", detail:"Jump to Projects, Presentations, Kosik, or Pictures." },
  { id:"pin", name:"Pin current folder", hint:"", detail:"Add the active pane's location to favorites." },
  { id:"sort", name:"Change sorting", hint:"Ctrl Shift S", detail:"Name, size, or modified date." },
  { id:"balance", name:"Balance panes", hint:"", detail:"Restore equal pane widths." },
  { id:"code", name:"Open folder in VS Code", hint:"", detail:"UI-only launch preview. No app will open." },
  { id:"copilot", name:"Start Copilot App session", hint:"", detail:"UI-only preview. Integration not verified." },
  { id:"answer", name:"Web answer / AI demo", hint:"", detail:"Example source-card presentation. No live search." },
  { id:"voice", name:"Voice input / UI preview", hint:"", detail:"No microphone access. Explore the transcript state." },
  { id:"reset", name:"Reset mock workspace", hint:"", detail:"Restore the original synthetic files and selections." },
];
const folder = (name) => ({ name, type:"folder", size:0, date:"2026-10-01" });
const file = (name, size, date="2026-10-05") => ({ name, type:"file", size, date });
export function fixture() {
  return {
    [ROOT]: ["Projects","Presentations","Kosik","Pictures","Archive","Empty","Restricted"].map(folder),
    [`${ROOT}\\Projects`]: [folder("TomasCommander"),folder("Hackathon"),folder("Research"),file("roadmap.md",4800),file("ideas.txt",2100),file("budget.xlsx",28600),file("meeting-notes.md",8200),file("README.md",1200),file("design-reference.pdf",2100000)],
    [`${ROOT}\\Projects\\TomasCommander`]: [file("PRD.md",6400),file("ui-exploration.md",5100),folder("prototypes")],
    [`${ROOT}\\Projects\\TomasCommander\\prototypes`]: [file("index.html",4200)],
    [`${ROOT}\\Projects\\Hackathon`]: [file("agenda.md",3400),file("participants.csv",7900)],
    [`${ROOT}\\Projects\\Research`]: [file("sources.md",2900)],
    [`${ROOT}\\Presentations`]: [file("Penta-hackathon-2026.pptx",8400000),file("Kosik-platform-overview.pptx",6200000),file("Foundry-demo.pdf",3400000),file("speaker-notes.md",7800)],
    [`${ROOT}\\Kosik`]: [folder("Presentations"),file("project-brief.md",4900),file("architecture.pdf",1500000)],
    [`${ROOT}\\Kosik\\Presentations`]: [],
    [`${ROOT}\\Pictures`]: [folder("Dovolená Srbsko"),folder("Praha"),file("workspace.png",2400000)],
    [`${ROOT}\\Pictures\\Dovolená Srbsko`]: [file("Belehrad-01.jpg",4300000),file("Dunaj-02.jpg",3900000),file("Novi-Sad-03.jpg",5100000)],
    [`${ROOT}\\Pictures\\Praha`]: [file("Karluv-most.jpg",3200000)],
    [`${ROOT}\\Archive`]: [file("notes-2025.md",4300)],
    [`${ROOT}\\Empty`]: [],
    [`${ROOT}\\Restricted`]: [],
  };
}
export function createState() {
  const pane = (path) => ({ path, filter:"", sort:"name", focused:0, anchor:null, selected:new Set() });
  return { files:fixture(), panes:[pane(`${ROOT}\\Projects`),pane(`${ROOT}\\Kosik`)], active:0, width:50,
    favorites:[`${ROOT}\\Projects`,`${ROOT}\\Presentations`,`${ROOT}\\Kosik`,`${ROOT}\\Pictures`], activity:[], revision:0 };
}
export const normalize = (text) => text.normalize("NFD").replace(/\p{Diacritic}/gu,"").toLowerCase();
export function entries(state, index) {
  const pane = state.panes[index];
  return (state.files[pane.path] ?? []).filter(item => normalize(item.name).includes(normalize(pane.filter)))
    .toSorted((a,b) => {
      if (a.type !== b.type) return a.type === "folder" ? -1 : 1;
      if (pane.sort === "size") return b.size-a.size || a.name.localeCompare(b.name);
      if (pane.sort === "date") return b.date.localeCompare(a.date) || a.name.localeCompare(b.name);
      return a.name.localeCompare(b.name);
    });
}
export function navigate(state, index, path) {
  if (!Object.hasOwn(state.files,path)) throw new Error("Mock folder no longer exists.");
  Object.assign(state.panes[index],{path, filter:"", focused:0, anchor:null, selected:new Set()});
  state.active=index;
}
export function parent(path) { return path === ROOT ? ROOT : path.slice(0,path.lastIndexOf("\\")); }
export function toggle(state, index, name) {
  const set=state.panes[index].selected;
  if (set.has(name)) set.delete(name); else set.add(name);
}
export function planOperation(state, kind) {
  if (!["copy","move","delete"].includes(kind)) throw new Error("Unsupported operation.");
  const pane=state.panes[state.active];
  const rows=entries(state,state.active);
  const names=pane.selected.size ? [...pane.selected] : rows[pane.focused] ? [rows[pane.focused].name] : [];
  if (!names.length) throw new Error("Select or focus an item first.");
  const source=pane.path, destination=kind==="delete" ? null : state.panes[1-state.active].path;
  if (destination===source) throw new Error("Source and destination are the same folder. Choose another destination.");
  for (const name of names) {
    if (!(state.files[source]??[]).some(item=>item.name===name)) throw new Error("A selected item no longer exists.");
    const itemPath=`${source}\\${name}`;
    if (destination && (destination===itemPath || destination.startsWith(`${itemPath}\\`)))
      throw new Error("Cannot copy or move a folder into itself.");
    if (destination && state.files[destination].some(item=>normalize(item.name)===normalize(name)))
      throw new Error(`Conflict: ${name} already exists at the destination. Nothing has changed.`);
  }
  if (source.endsWith("\\Restricted") || destination?.endsWith("\\Restricted"))
    throw new Error("Simulated permission denied. No changes made.");
  return {kind,source,destination,names,revision:state.revision};
}
export function executePlan(state, plan) {
  if (state.revision!==plan.revision) throw new Error("Workspace changed. Create a new plan before approving.");
  if (!Object.hasOwn(state.files,plan.source) || (plan.destination && !Object.hasOwn(state.files,plan.destination)))
    throw new Error("A target folder no longer exists. Create a new plan.");
  for (const name of plan.names) {
    if (!state.files[plan.source].some(item=>item.name===name)) throw new Error("An item no longer exists. Create a new plan.");
    if (plan.destination && state.files[plan.destination].some(item=>normalize(item.name)===normalize(name)))
      throw new Error("Destination changed. Nothing has changed.");
  }
  for (const name of plan.names) {
    const item=state.files[plan.source].find(item=>item.name===name);
    const sourcePath=`${plan.source}\\${name}`;
    const descendants=Object.keys(state.files).filter(path=>path===sourcePath || path.startsWith(`${sourcePath}\\`));
    if (plan.kind!=="delete") {
      state.files[plan.destination].push({...item});
      for (const path of descendants) state.files[`${plan.destination}\\${name}${path.slice(sourcePath.length)}`]=state.files[path].map(item=>({...item}));
    }
    if (plan.kind!=="copy") {
      state.files[plan.source]=state.files[plan.source].filter(item=>item.name!==name);
      for (const path of descendants) delete state.files[path];
    }
  }
  state.revision++;
  for (const pane of state.panes) {
    if (!Object.hasOwn(state.files,pane.path)) pane.path=ROOT;
    pane.selected.clear(); pane.focused=0; pane.anchor=null;
  }
  state.activity.unshift(`${plan.kind}: ${plan.names.length} item(s) / simulated`);
  state.activity=state.activity.slice(0,5);
}
export function search(state, query, ai=false) {
  const words=normalize(query).split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  return Object.entries(state.files).flatMap(([path,items]) => items.filter(item=>{
    const text=normalize(`${path}\\${item.name}`);
    return ai ? item.type==="file" && /\.(jpg|png)$/i.test(item.name) && text.includes("srbsko")
      : words.every(word=>text.includes(word));
  }).map(item=>({...item,path,fullPath:`${path}\\${item.name}`})));
}
