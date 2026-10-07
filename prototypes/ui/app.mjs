import { ROOT,DESIGNS,COMMANDS,createState,entries,navigate,parent,toggle,planOperation,executePlan,search,normalize } from "./model.mjs";

let state=createState();
let design="commander";
let returnFocus=null;
let noteDesign=design;
const $ = (selector) => document.querySelector(selector);
const esc = (value) => String(value).replace(/[&<>"']/g,char=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[char]));
const shortPath = (path) => path.replace(ROOT,"~");
const baseName = (path) => path.slice(path.lastIndexOf("\\")+1);
const sizeText = (size) => size>=1e6 ? `${(size/1e6).toFixed(1)} MB` : size>=1e3 ? `${(size/1e3).toFixed(1)} KB` : `${size} B`;
let notes={};
try { notes=JSON.parse(localStorage.getItem("tc-ui-notes")??"{}"); if (!notes || typeof notes!=="object" || Array.isArray(notes)) throw new Error("Invalid notes format."); }
catch (error) { $("#status").textContent=`Notes storage could not be loaded: ${error.message}`; notes={}; }

function tell(message) { $("#status").textContent=message; }
function focusList(index=state.active) { state.active=index; render(); $(`#list-${index}`).focus(); }
function render() {
  $("#designs").innerHTML=DESIGNS.map(item=>`<button data-design-choice="${item.id}" aria-pressed="${design===item.id}"><span>${item.category}</span>${item.name}</button>`).join("");
  $("#design-description").textContent=DESIGNS.find(item=>item.id===design).description;
  $("#context-count").textContent=`${Object.values(state.files).flat().filter(item=>item.type==="file").length} mock files / 2 panes`;
  $("#places-list").innerHTML=state.favorites.map(path=>`<button data-path="${esc(path)}">${esc(baseName(path))}</button>`).join("");
  state.panes.forEach((pane,index)=>{
    const rows=entries(state,index);
    pane.focused=Math.max(0,Math.min(pane.focused,rows.length-1));
    const element=$(index===0?"#pane-left":"#pane-right");
    element.dataset.active=String(state.active===index);
    element.innerHTML=`
      <div class="pane-top"><span class="pane-name">${index===0?"01 / LEFT PANE":"02 / RIGHT PANE"} ${state.active===index?"[active]":""}</span><div class="pane-tools"><button data-command="favorites" data-pane="${index}">Places</button><button data-command="pin" data-pane="${index}" aria-label="Pin ${index===0?"left":"right"} folder">+ pin</button></div></div>
      <div class="path-row"><button data-parent="${index}" aria-label="Parent folder in ${index===0?"left":"right"} pane">..</button><span class="path" title="${esc(pane.path)}">${esc(shortPath(pane.path))}</span></div>
      <div class="filter-row"><input id="filter-${index}" data-filter="${index}" aria-label="Filter ${index===0?"left":"right"} pane" placeholder="Filter this folder..." value="${esc(pane.filter)}"><select class="sort" id="sort-${index}" data-sort="${index}" aria-label="Sort ${index===0?"left":"right"} pane">${["name","size","date"].map(sort=>`<option value="${sort}" ${pane.sort===sort?"selected":""}>${sort}</option>`).join("")}</select></div>
      <div class="file-head" aria-hidden="true"><span></span><span>NAME</span><span>SIZE</span><span>MODIFIED</span></div>
      <div class="file-list" id="list-${index}" role="listbox" tabindex="0" aria-label="${index===0?"Left":"Right"} files" aria-multiselectable="true" ${rows.length?`aria-activedescendant="row-${index}-${pane.focused}"`:""}>
        ${rows.length?rows.map((item,row)=>`<div class="file-row" id="row-${index}-${row}" role="option" aria-selected="${pane.selected.has(item.name)}" data-row="${row}" data-pane="${index}" data-folder="${item.type==="folder"}" data-focused="${row===pane.focused}" data-selected="${pane.selected.has(item.name)}"><input type="checkbox" tabindex="-1" data-toggle="${row}" data-pane="${index}" aria-label="Select ${esc(item.name)}" ${pane.selected.has(item.name)?"checked":""}><span class="file-name" title="${esc(item.name)}"><span class="file-icon" aria-hidden="true">${item.type==="folder"?"[/]":"[.]"}</span>${esc(item.name)}</span><span class="file-size">${item.type==="folder"?"&lt;DIR&gt;":sizeText(item.size)}</span><span class="file-date">${item.date}</span></div>`).join(""):`<div class="empty">${pane.path.endsWith("\\Restricted")?"Simulated access denied.<br>Use .. to return.":pane.filter?"No matching files.<br>Clear the filter to see this folder.":"An empty folder.<br>Copy an item here to try the workflow."}</div>`}
      </div><div class="pane-foot"><span>${rows.length} visible · ${pane.selected.size} selected</span><span>${index===state.active?"READY / FOCUSED":"READY"}</span></div>`;
  });
  $("#file-actions").innerHTML=["copy","move","delete"].map(id=>{
    const command=COMMANDS.find(item=>item.id===id);
    return `<button data-command="${id}">${id[0].toUpperCase()+id.slice(1)} <kbd>${command.hint}</kbd></button>`;
  }).join("");
  const current=entries(state,state.active)[state.panes[state.active].focused];
  $("#inspection").innerHTML=`<h3>${esc(current?.name??"No item focused")}</h3><p>${current?current.type==="folder"?"Directory / Enter to browse":`${esc(sizeText(current.size))}<br>Modified ${esc(current.date)}`:"Navigate to a populated folder."}</p><p>${esc(shortPath(state.panes[state.active].path))}</p>`;
  $("#activity").innerHTML=state.activity.length?state.activity.map(text=>`<li>${esc(text)}</li>`).join(""):"<li>No operations yet.</li>";
  $("#panes").style.setProperty("--left",`${state.width}fr`);
  $("#panes").style.setProperty("--right",`${100-state.width}fr`);
  $("#divider").setAttribute("aria-valuenow",String(state.width));
}
function setDesign(id) {
  if (!DESIGNS.some(item=>item.id===id)) return;
  design=id; document.documentElement.dataset.design=id; render();
}
function openModal(title,label,html) {
  if (!$("#modal").open) returnFocus=document.activeElement;
  $("#modal-title").textContent=title; $("#modal-label").textContent=label;
  $("#modal-body").onclick=null;
  $("#modal-body").innerHTML=html;
  if (!$("#modal").open) $("#modal").showModal();
  const initial=$("#modal-body input, #modal-body textarea, #modal-body button");
  if (initial) initial.focus();
}
function closeModal(nextFocus) {
  if (nextFocus instanceof HTMLElement) returnFocus=nextFocus;
  $("#modal").close();
}
$("#modal").addEventListener("close",()=>{
  if (returnFocus?.isConnected) returnFocus.focus(); else $(`#list-${state.active}`).focus();
});
$("#modal-close").addEventListener("click",closeModal);
function errorDialog(error) {
  openModal("No changes made","MOCK OPERATION / ERROR",`<p class="dialog-note">${esc(error.message)}</p><div class="modal-actions"><button id="dismiss-error">Back to files</button></div>`);
  $("#dismiss-error").onclick=closeModal;
}
function preview(plan) {
  openModal(`${plan.kind[0].toUpperCase()+plan.kind.slice(1)} ${plan.names.length} item(s)`,`SIMULATED OPERATION / ${plan.kind==="copy"?"PREVIEW":"APPROVAL REQUIRED"}`,
    `<p class="dialog-note">This changes mock files only. Nothing on your computer will be touched.</p><div class="plan"><div>FROM ${esc(plan.source)}</div><div>${plan.destination?`TO ${esc(plan.destination)}`:"DELETE from in-memory demo"}</div><ul>${plan.names.map(name=>`<li>${esc(name)}</li>`).join("")}</ul></div><div class="modal-actions"><button id="cancel-plan">Cancel</button><button class="accent-button" id="approve-plan">${plan.kind==="copy"?"Run simulated copy":`Approve simulated ${plan.kind}`}</button></div>`);
  $("#cancel-plan").onclick=closeModal;
  $("#approve-plan").onclick=()=>{
    try { executePlan(state,plan); render(); tell(`Simulated ${plan.kind} complete: ${plan.names.length} item(s). No disk access.`); closeModal(); }
    catch(error) { errorDialog(error); }
  };
  // Destructive approval is deliberate, never the dialog's initial Enter target.
  if (plan.kind!=="copy") $("#cancel-plan").focus();
}
function locations() {
  openModal("Favorite locations","JUMP / ACTIVE PANE",`<p class="dialog-note">Jump the active pane to a pinned folder.</p><div class="command-list">${state.favorites.map(path=>`<button data-favorite="${esc(path)}"><span>${esc(baseName(path))}<small>${esc(path)}</small></span><span>/</span></button>`).join("")}</div>`);
  $("#modal-body").onclick=event=>{
    const button=event.target.closest("[data-favorite]");
    if (!button) return;
    try { navigate(state,state.active,button.dataset.favorite); render(); closeModal($(`#list-${state.active}`)); }
    catch(error) { errorDialog(error); }
  };
}
function findFiles() {
  let ai=false;
  openModal("Find a file","MOCK SEARCH",`<input id="search-input" class="dialog-input" aria-label="Search mock files" placeholder="Try Penta, roadmap, or Srbsko"><div class="dialog-note">Deterministic name/path search across the mock tree. Or explore a fixed AI-result example:</div><p><button id="ai-search">AI demo: images from Dovolene, Srbsko</button></p><div id="search-state" class="dialog-note" role="status"></div><div id="search-results" class="command-list"></div>`);
  const update=()=>{
    const query=$("#search-input").value;
    const found=search(state,query,ai);
    $("#search-state").textContent=ai?"Fixed AI UI example / not a live model. Match explanation: image files in the Serbian holiday folder.":query?`${found.length} matching items.`:"Type to search. No real files will be read.";
    $("#search-results").innerHTML=found.map((item,index)=>`<button data-search-result="${index}"><span>${esc(item.name)}<small>${esc(shortPath(item.fullPath))}</small></span><span>Reveal →</span></button>`).join("");
    $("#search-results").onclick=event=>{
      const button=event.target.closest("[data-search-result]");
      if (!button) return;
      const item=found[Number(button.dataset.searchResult)];
      navigate(state,state.active,item.path);
      state.panes[state.active].focused=entries(state,state.active).findIndex(row=>row.name===item.name);
      render(); closeModal($(`#list-${state.active}`));
    };
  };
  $("#search-input").oninput=()=>{ai=false;update();};
  $("#ai-search").onclick=()=>{
    ai=true; $("#search-input").value="images from Dovolene, Srbsko";
    $("#search-state").textContent="Loading a fixed demo result...";
    $("#search-results").innerHTML="";
    const input=$("#search-input");
    setTimeout(()=>{ if ($("#modal").open && $("#search-input")===input) update(); },450);
  };
  update();
}
function demoIntent(text) {
  if (!normalize(text).includes("penta") || !normalize(text).includes("kosik")) {
    tell("AI UI demo supports the Penta-to-Kosik example only; no live model is connected.");
    return;
  }
  const source=`${ROOT}\\Presentations`,destination=`${ROOT}\\Kosik`;
  const item=state.files[source].find(item=>normalize(item.name).includes("penta"));
  if (!item) { errorDialog(new Error("The demo presentation is no longer at its original location. Reset the demo.")); return; }
  if (state.files[destination].some(row=>normalize(row.name)===normalize(item.name))) {
    errorDialog(new Error("The presentation already exists in Kosik. No overwrite is allowed in this demo.")); return;
  }
  preview({kind:"copy",source,destination,names:[item.name],revision:state.revision});
}
function palette() {
  openModal("What would you like to do?","COMMAND PALETTE / MOCK WORKSPACE",
    `<input id="palette-input" class="dialog-input" aria-label="Command or demo intent" placeholder="Search a command or describe an action..."><div id="palette-results" class="command-list"></div><p class="dialog-note">AI UI demo: “Copy my Penta hackathon presentation to Kosik.” Only this fixed intent is supported. No model is connected.</p>`);
  const update=()=>{
    const query=normalize($("#palette-input").value);
    const matches=COMMANDS.filter(item=>normalize(`${item.name} ${item.detail}`).includes(query));
    $("#palette-results").innerHTML=matches.map(item=>`<button data-palette-command="${item.id}"><span>${esc(item.name)}<small>${esc(item.detail)}</small></span><kbd>${esc(item.hint||"Enter")}</kbd></button>`).join("")
      + (query.includes("penta") && query.includes("kosik") ? `<button id="intent-demo"><span>Preview Penta → Kosik copy<small>Fixed AI UI example / synthetic files</small></span><kbd>Enter</kbd></button>`:"")
      + (!matches.length && !(query.includes("penta")&&query.includes("kosik"))?`<p class="dialog-note">No matching command. This prototype does not interpret arbitrary prompts.</p>`:"");
    $("#intent-demo")?.addEventListener("click",()=>demoIntent($("#palette-input").value));
  };
  $("#palette-input").oninput=update;
  $("#palette-input").onkeydown=event=>{
    if (event.key==="ArrowDown") {event.preventDefault();$("#palette-results button")?.focus();}
    if (event.key==="Enter") {event.preventDefault();$("#palette-results button")?.click();}
  };
  $("#palette-results").onclick=event=>{
    const button=event.target.closest("[data-palette-command]");
    if (button) run(button.dataset.paletteCommand);
  };
  update();
}
function answer() {
  openModal("A compact answer, with sources","AI ANSWER / STATIC UI EXAMPLE",
    `<p class="dialog-note">This is fictional example content, not a search result. No WorkIQ, WebIQ, or model is connected.</p><p>A useful file workspace makes commands predictable, keeps the active pane obvious, and previews consequential actions before execution.</p><div class="command-list"><button id="source-one"><span>01 / Keyboard-first interaction<small>Example article summary: make focus and command hints visible.</small></span><span>Preview →</span></button><button id="source-two"><span>02 / Safer file operations<small>Example article summary: show source, destination, and conflicts.</small></span><span>Preview →</span></button></div>`);
  for (const id of ["source-one","source-two"]) $(`#${id}`).onclick=()=>tell("Example source selected. No real article or browser launch exists in this UI demo.");
}
function launchPreview(id) {
  openModal(id==="code"?"Open in VS Code":"Start Copilot session","EXTERNAL APP / UI PREVIEW",
    `<div class="plan">${esc(state.panes[state.active].path)}</div><p class="dialog-note">No external application will be launched. ${id==="copilot"?"Supported session-launch integration still needs verification.":"The product adapter will eventually open this folder in VS Code."}</p><div class="modal-actions"><button id="preview-close">Back to files</button></div>`);
  $("#preview-close").onclick=closeModal;
}
function run(id) {
  try {
    if (["copy","move","delete"].includes(id)) return preview(planOperation(state,id));
    if (id==="palette") return palette();
    if (id==="favorites") return locations();
    if (id==="search") return findFiles();
    if (id==="answer") return answer();
    if (["code","copilot"].includes(id)) return launchPreview(id);
    if (id==="voice") {
      openModal("Review a transcript","VOICE / UI PREVIEW",`<p class="dialog-note">No recording or microphone access. This is an editable example transcript.</p><textarea id="transcript" aria-label="Example transcript">Copy my Penta hackathon presentation to Kosik</textarea><div class="modal-actions"><button id="transcript-run">Preview demo intent</button></div>`);
      $("#transcript-run").onclick=()=>demoIntent($("#transcript").value); return;
    }
    if (id==="pin") {
      const path=state.panes[state.active].path;
      if (!state.favorites.includes(path)) state.favorites.push(path);
      render(); tell(`Pinned ${shortPath(path)} in this demo.`); if ($("#modal").open) closeModal(); return;
    }
    if (id==="sort") {
      if ($("#modal").open) closeModal($(`#sort-${state.active}`));
      $(`#sort-${state.active}`).focus(); tell("Choose name, size, or date sorting."); return;
    }
    if (id==="balance") {state.width=50;render();tell("Pane widths balanced.");if ($("#modal").open) closeModal();return;}
    if (id==="reset") {state=createState();render();tell("Original mock workspace restored. Design notes are kept.");if ($("#modal").open) closeModal();return;}
    throw new Error("Unknown command.");
  } catch(error) { errorDialog(error); }
}
function openFocused(index) {
  const item=entries(state,index)[state.panes[index].focused];
  if (!item) return;
  if (item.type==="folder") {
    navigate(state,index,`${state.panes[index].path}\\${item.name}`); focusList(index);
  } else {
    openModal(item.name,"FILE / MOCK PREVIEW",`<div class="plan">${esc(state.panes[index].path)}\\${esc(item.name)}<br>${sizeText(item.size)} / ${item.date}</div><p class="dialog-note">Synthetic file metadata only. No document content is read or opened.</p><div class="modal-actions"><button id="file-preview-close">Back to files</button></div>`);
    $("#file-preview-close").onclick=closeModal;
  }
}
document.addEventListener("click",event=>{
  const choice=event.target.closest("[data-design-choice]");
  if (choice) return setDesign(choice.dataset.designChoice);
  const command=event.target.closest("[data-command]");
  if (command) {
    if (command.dataset.pane!==undefined) state.active=Number(command.dataset.pane);
    return run(command.dataset.command);
  }
  const place=event.target.closest("[data-path]");
  if (place) {
    try {navigate(state,state.active,place.dataset.path);return focusList();}
    catch(error) {return errorDialog(error);}
  }
  const up=event.target.closest("[data-parent]");
  if (up) {const index=Number(up.dataset.parent);navigate(state,index,parent(state.panes[index].path));return focusList(index);}
  const row=event.target.closest("[data-row]");
  if (row) {
    const index=Number(row.dataset.pane),position=Number(row.dataset.row),pane=state.panes[index];
    const rows=entries(state,index);
    if (event.target.matches("[data-toggle]") || event.ctrlKey) toggle(state,index,rows[position].name);
    else if (event.shiftKey) {
      const start=pane.anchor??pane.focused;
      for (let i=Math.min(start,position);i<=Math.max(start,position);i++) pane.selected.add(rows[i].name);
    }
    if (!event.shiftKey) pane.anchor=position;
    pane.focused=position; state.active=index; focusList(index);
  }
});
document.addEventListener("dblclick",event=>{
  const row=event.target.closest("[data-row]");
  if (row && !event.target.matches("input")) openFocused(Number(row.dataset.pane));
});
document.addEventListener("focusin",event=>{
  const pane=event.target.closest(".pane");
  if (pane) {
    state.active=pane.id==="pane-left"?0:1;
    document.querySelectorAll(".pane").forEach((element,index)=>element.dataset.active=String(index===state.active));
  }
});
document.addEventListener("input",event=>{
  if (event.target.matches("[data-filter]")) {
    const index=Number(event.target.dataset.filter),value=event.target.value,start=event.target.selectionStart;
    state.active=index; state.panes[index].filter=value; state.panes[index].focused=0;
    render(); const input=$(`#filter-${index}`);input.focus();input.setSelectionRange(start,start);
  }
});
document.addEventListener("change",event=>{
  if (event.target.matches("[data-sort]")) {
    const index=Number(event.target.dataset.sort);
    state.active=index; state.panes[index].sort=event.target.value;state.panes[index].focused=0;
    render();$(`#sort-${index}`).focus();
  }
});
document.addEventListener("keydown",event=>{
  const key=event.key.toLowerCase();
  if (event.ctrlKey && event.shiftKey && key==="p") {event.preventDefault();return palette();}
  if ($("#modal").open) {
    if (["ArrowDown","ArrowUp"].includes(event.key) && event.target.closest(".command-list")) {
      event.preventDefault();
      const buttons=[...event.target.closest(".command-list").querySelectorAll("button")];
      const index=buttons.indexOf(event.target.closest("button"));
      buttons[(index+(event.key==="ArrowDown"?1:-1)+buttons.length)%buttons.length]?.focus();
    }
    return;
  }
  const editing=event.target.matches("input,textarea,select");
  if (editing) return;
  if (event.ctrlKey && event.shiftKey) {
    const command={c:"copy",m:"move",s:"sort",f:"search"}[key];
    if (command) {event.preventDefault();return run(command);}
    if (["ArrowLeft","ArrowRight"].includes(event.key) && event.target.closest(".pane")) {
      event.preventDefault();state.width=Math.max(30,Math.min(70,state.width+(event.key==="ArrowRight"?5:-5)));return focusList();
    }
  }
  const list=event.target.closest(".file-list");
  if (!list) return;
  const index=Number(list.id.slice(-1)),pane=state.panes[index],rows=entries(state,index);
  if (event.key==="Tab" && !event.shiftKey) {event.preventDefault();return focusList(1-index);}
  if (["ArrowDown","ArrowUp","Home","End"].includes(event.key)) {
    event.preventDefault();
    const previous=pane.focused;
    pane.focused=event.key==="Home"?0:event.key==="End"?rows.length-1:Math.max(0,Math.min(rows.length-1,previous+(event.key==="ArrowDown"?1:-1)));
    if (event.shiftKey && rows.length) {
      const start=pane.anchor??previous;pane.anchor=start;
      for (let i=Math.min(start,pane.focused);i<=Math.max(start,pane.focused);i++) pane.selected.add(rows[i].name);
    } else pane.anchor=pane.focused;
    focusList(index);$(`#row-${index}-${pane.focused}`)?.scrollIntoView({block:"nearest"});return;
  }
  if (event.key===" ") {event.preventDefault();if(rows[pane.focused])toggle(state,index,rows[pane.focused].name);return focusList(index);}
  if (event.key==="Enter") {event.preventDefault();return openFocused(index);}
  if (event.key==="Backspace") {event.preventDefault();navigate(state,index,parent(pane.path));return focusList(index);}
  if (event.key==="Delete") {event.preventDefault();return run("delete");}
  if (event.key==="Escape") {pane.selected.clear();return focusList(index);}
  if (event.ctrlKey && key==="a") {event.preventDefault();rows.forEach(item=>pane.selected.add(item.name));return focusList(index);}
  if (event.ctrlKey && key==="f") {event.preventDefault();$(`#filter-${index}`).focus();}
});
$("#divider").onkeydown=event=>{
  if (!["ArrowLeft","ArrowRight","Home"].includes(event.key)) return;
  event.preventDefault();
  state.width=event.key==="Home"?50:Math.max(30,Math.min(70,state.width+(event.key==="ArrowRight"?5:-5)));
  render();$("#divider").focus();
};
$("#divider").onpointerdown=event=>{
  const divider=event.currentTarget;
  divider.setPointerCapture(event.pointerId);
  divider.onpointermove=move=>{
    if (!divider.hasPointerCapture(move.pointerId)) return;
    const rect=$("#panes").getBoundingClientRect();
    state.width=Math.max(30,Math.min(70,Math.round((move.clientX-rect.left)/rect.width*100)));
    $("#panes").style.setProperty("--left",`${state.width}fr`);
    $("#panes").style.setProperty("--right",`${100-state.width}fr`);
    divider.setAttribute("aria-valuenow",String(state.width));
  };
  divider.onpointerup=up=>{divider.releasePointerCapture(up.pointerId);divider.onpointermove=null;};
};
$("#theme").onclick=()=>{
  const dark=document.documentElement.dataset.theme==="dark";
  document.documentElement.dataset.theme=dark?"light":"dark";
  $("#theme").textContent=dark?"Dark mode":"Light mode";
  $("#theme").setAttribute("aria-label",`Switch to ${dark?"dark":"light"} theme`);
};
const accents=[{id:"blue",name:"blue"},{id:"red",name:"red-orange"},{id:"green",name:"green"},{id:"yellow",name:"yellow"}];
$("#accent").onclick=()=>{
  const current=accents.findIndex(item=>item.id===document.documentElement.dataset.accent);
  const next=accents[(current+1)%accents.length],following=accents[(current+2)%accents.length];
  document.documentElement.dataset.accent=next.id;
  $("#accent").textContent=`Accent: ${next.name}`;
  $("#accent").setAttribute("aria-label",`Accent ${next.name}; switch to ${following.name}`);
};
function saveNotes() {
  notes[noteDesign]={...notes[noteDesign],text:$("#design-notes").value,updatedAt:new Date().toISOString()};
  try {localStorage.setItem("tc-ui-notes",JSON.stringify(notes));$("#note-status").textContent="Saved in this browser only. Export and attach the file to share it with the agent.";}
  catch(error) {$("#note-status").textContent=`Local save failed: ${error.message}. Export before closing.`;}
}
function feedback() {
  openModal("Compare. Keep. Change.","DESIGN FEEDBACK / ROUND 01",
    `<p class="dialog-note">Tell me what to keep, remove, or combine. Notes stay in this browser; they are not automatically sent to the agent.</p><div class="feedback-tabs">${DESIGNS.map(item=>`<button data-note-design="${item.id}" aria-pressed="${noteDesign===item.id}">${item.name}</button>`).join("")}</div><label for="design-notes">Notes for <strong id="note-name"></strong></label><textarea id="design-notes" placeholder="Keep... / Change... / Keyboard issues..."></textarea><p class="feedback-status" id="note-status" role="status"></p><div class="modal-actions"><button id="prefer-design">Mark as preferred</button><button class="accent-button" id="export-notes">Export feedback JSON</button></div>`);
  const update=()=>{
    $("#note-name").textContent=DESIGNS.find(item=>item.id===noteDesign).name;
    $("#design-notes").value=notes[noteDesign]?.text??"";
    document.querySelectorAll("[data-note-design]").forEach(button=>button.setAttribute("aria-pressed",String(button.dataset.noteDesign===noteDesign)));
    $("#prefer-design").textContent=notes[noteDesign]?.preferred?"Preferred design":"Mark as preferred";
  };
  document.querySelectorAll("[data-note-design]").forEach(button=>button.onclick=()=>{saveNotes();noteDesign=button.dataset.noteDesign;update();});
  $("#design-notes").oninput=saveNotes;
  $("#prefer-design").onclick=()=>{
    for (const item of DESIGNS) notes[item.id]={...notes[item.id],preferred:item.id===noteDesign};
    saveNotes();update();
  };
  $("#export-notes").onclick=()=>{
    saveNotes();
    const blob=new Blob([JSON.stringify({round:1,exportedAt:new Date().toISOString(),notes},null,2)],{type:"application/json"});
    const url=URL.createObjectURL(blob),link=document.createElement("a");
    link.href=url;link.download="tomascommander-ui-feedback-round-01.json";link.click();
    setTimeout(()=>URL.revokeObjectURL(url),1000);
    $("#note-status").textContent="Feedback export requested. Attach the downloaded JSON here; the agent does not receive browser notes automatically.";
  };
  update();
}
$("#feedback-open").onclick=()=>{noteDesign=design;feedback();};
render();
