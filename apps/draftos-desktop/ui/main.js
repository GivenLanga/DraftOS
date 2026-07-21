// DraftOS shell logic. Thin client: all real work happens in Rust commands.

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

const $ = (id) => document.getElementById(id);

const CLAUSE_TYPES = [
  'Definitions', 'Termination', 'Confidentiality', 'Payment',
  'Intellectual Property', 'Indemnity', 'Warranties', 'Force Majeure',
  'Dispute Resolution', 'Governing Law', 'Notices', 'Breach',
  'Restraint of Trade', 'Assignment', 'Limitation of Liability',
  'Data Protection', 'Insurance', 'General',
];

let statusTimer = null;

function status(text, isError = false) {
  const el = $('status');
  el.innerHTML = '';
  const line = document.createElement('div');
  line.textContent = text;
  if (isError) line.className = 'err';
  el.appendChild(line);
  clearTimeout(statusTimer);
  statusTimer = setTimeout(() => { el.innerHTML = ''; }, 8000);
}

// ---------- Library ----------

async function refreshSources() {
  const sources = await invoke('list_sources');
  const list = $('sources');
  list.innerHTML = '';
  $('library-empty').hidden = sources.length > 0;

  const sourceFilter = $('filter-source');
  const selected = sourceFilter.value;
  sourceFilter.innerHTML = '<option value="">All sources</option>';

  for (const s of sources) {
    const li = document.createElement('li');
    li.className = 'source ' + (s.attached ? 'attached' : 'detached');

    const name = document.createElement('div');
    name.className = 'source-name';
    name.textContent = s.name;

    const meta = document.createElement('div');
    meta.className = 'source-meta';
    meta.textContent = `${s.documents} docs · ${s.clauses} clauses\n${s.folder}`;
    meta.style.whiteSpace = 'pre-line';

    const actions = document.createElement('div');
    actions.className = 'source-actions';
    actions.append(
      action(s.attached ? 'Detach' : 'Attach', async () => {
        await invoke('set_attached', { name: s.name, attached: !s.attached });
        status(s.attached
          ? `Detached ${s.name} — index kept, re-attach any time`
          : `Re-attached ${s.name}`);
        refreshSources();
      }),
      action('Rescan', async () => {
        await invoke('rescan_source', { name: s.name });
        status(`Rescanning ${s.name}…`);
      }),
      action('Remove', async () => {
        if (!confirm(`Remove '${s.name}' and delete its index? The original files are not touched.`)) return;
        await invoke('remove_source', { name: s.name });
        status(`Removed ${s.name}`);
        refreshSources();
      }),
    );

    li.append(name, meta, actions);
    list.appendChild(li);

    if (s.attached) {
      const opt = document.createElement('option');
      opt.value = s.name;
      opt.textContent = s.name;
      sourceFilter.appendChild(opt);
    }
  }
  sourceFilter.value = selected;
  renderDraftScope(sources);
}

function action(label, handler) {
  const b = document.createElement('button');
  b.type = 'button';
  b.textContent = label;
  b.addEventListener('click', (e) => {
    e.stopPropagation();
    handler().catch((err) => status(String(err), true));
  });
  return b;
}

$('browse-folder').addEventListener('click', async () => {
  try {
    const folder = await invoke('pick_folder');
    if (!folder) return; // cancelled
    $('add-folder').value = folder;
    // Suggest a name from the folder's own basename, unless one's already typed.
    if (!$('add-name').value.trim()) {
      const base = folder.replace(/[/\\]+$/, '').split(/[/\\]/).pop();
      if (base) $('add-name').value = base;
    }
    $('add-name').focus();
  } catch (err) {
    status(String(err), true);
  }
});

$('add-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const folder = $('add-folder').value.trim();
  const name = $('add-name').value.trim();
  if (!folder || !name) { status('Enter a folder path and a name', true); return; }
  try {
    await invoke('add_source', { folder, name });
    $('add-folder').value = '';
    $('add-name').value = '';
    status(`Attached ${name} — indexing in the background…`);
    refreshSources();
  } catch (err) {
    status(String(err), true);
  }
});

// ---------- Search ----------

async function runSearch() {
  const query = $('query').value.trim();
  if (!query) return;
  const hits = await invoke('search_clauses', {
    query,
    kind: $('filter-kind').value || null,
    clauseType: $('filter-type').value || null,
    contractType: null,
    source: $('filter-source').value || null,
    limit: 20,
  });
  renderResults(hits, query);
}

function renderResults(hits, query) {
  const results = $('results');
  results.innerHTML = '';

  if (hits.length === 0) {
    const p = document.createElement('p');
    p.className = 'no-results';
    p.textContent = `Nothing matched “${query}”. Try broader words, clear the filters, or check that a source is attached.`;
    results.appendChild(p);
    return;
  }

  const count = document.createElement('p');
  count.className = 'result-count';
  count.textContent = `${hits.length} clause${hits.length === 1 ? '' : 's'} retrieved`;
  results.appendChild(count);

  for (const h of hits) {
    results.appendChild(renderHit(h));
  }
}

function renderHit(h) {
  const card = document.createElement('article');
  card.className = 'hit';
  card.tabIndex = 0;

  const head = document.createElement('div');
  head.className = 'hit-head';

  if (h.number) {
    const num = document.createElement('span');
    num.className = 'hit-number';
    num.textContent = h.number;
    head.appendChild(num);
  }

  const title = document.createElement('h3');
  title.className = 'hit-title';
  title.textContent = h.term ? `“${h.term}”` : (h.heading || '(untitled)');
  head.appendChild(title);

  const kind = document.createElement('span');
  kind.className = 'hit-kind';
  kind.textContent = h.kind;
  head.appendChild(kind);

  const body = document.createElement('div');
  body.className = 'hit-body';
  body.textContent = h.body;

  const prov = document.createElement('div');
  prov.className = 'hit-prov';
  const tags = [
    `${h.source_name} / ${h.file}`,
    h.metadata.clause_type,
    h.metadata.contract_type,
    h.metadata.jurisdiction,
  ].filter(Boolean);
  for (const t of tags) {
    const span = document.createElement('span');
    span.className = 'tag';
    span.textContent = t;
    prov.appendChild(span);
  }

  card.append(head, body, prov);
  const toggle = () => card.classList.toggle('open');
  card.addEventListener('click', toggle);
  card.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggle(); }
  });
  return card;
}

$('search-form').addEventListener('submit', (e) => {
  e.preventDefault();
  runSearch().catch((err) => status(String(err), true));
});
for (const id of ['filter-kind', 'filter-type', 'filter-source']) {
  $(id).addEventListener('change', () => {
    if ($('query').value.trim()) runSearch().catch((err) => status(String(err), true));
  });
}

// ---------- Model & Assistant ----------

let currentModel = null; // ModelView from Rust, or null

function renderModelState() {
  const summary = $('model-summary');
  const banner = $('privacy-banner');
  if (!currentModel) {
    summary.textContent = 'No model — search & drafting work fully offline.';
    summary.classList.remove('cloud');
    banner.textContent = 'No model configured — pick one under Model in the sidebar to use the assistant.';
    banner.className = 'privacy-banner none';
    return;
  }
  const label = `${currentModel.adapter}${currentModel.model ? ' · ' + currentModel.model : ''}`;
  if (currentModel.local) {
    summary.textContent = `${label} — local`;
    summary.classList.remove('cloud');
    banner.textContent = 'Local model — nothing leaves this machine.';
    banner.className = 'privacy-banner local';
  } else {
    summary.textContent = `${label} — CLOUD`;
    summary.classList.add('cloud');
    banner.textContent = `CLOUD MODEL ACTIVE — your questions and retrieved clause text are sent to ${currentModel.adapter}.`;
    banner.className = 'privacy-banner cloud';
  }
}

async function refreshModel() {
  currentModel = await invoke('get_model');
  if (currentModel) {
    $('model-adapter').value = currentModel.adapter;
    $('model-name').value = currentModel.model || '';
    $('model-url').value = currentModel.base_url || '';
  }
  renderModelState();
}

$('model-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  try {
    currentModel = await invoke('set_model', {
      adapter: $('model-adapter').value,
      model: $('model-name').value.trim() || null,
      baseUrl: $('model-url').value.trim() || null,
    });
    renderModelState();
    status(`Model set: ${currentModel.adapter}${currentModel.local ? ' (local)' : ' (cloud)'}`);
  } catch (err) {
    status(String(err), true);
  }
});

$('model-clear').addEventListener('click', async () => {
  try {
    await invoke('clear_model');
    currentModel = null;
    renderModelState();
    status('Model unset — DraftOS keeps working offline.');
  } catch (err) {
    status(String(err), true);
  }
});

$('model-key-save').addEventListener('click', async () => {
  const key = $('model-key').value;
  if (!key.trim()) { status('Paste a key first', true); return; }
  try {
    await invoke('store_model_key', { adapter: $('model-adapter').value, key });
    $('model-key').value = '';
    status('Key stored in the OS keychain.');
  } catch (err) {
    status(String(err), true);
  }
});

// Tabs
const VIEWS = ['search', 'draft', 'assistant'];
const FOCUS = { search: 'query', draft: 'draft-type', assistant: 'ask-input' };
function showView(which) {
  for (const name of VIEWS) {
    const on = name === which;
    $('view-' + name).hidden = !on;
    $('tab-' + name).classList.toggle('active', on);
    $('tab-' + name).setAttribute('aria-selected', String(on));
  }
  const f = $(FOCUS[which]);
  if (f) f.focus();
}
for (const name of VIEWS) {
  $('tab-' + name).addEventListener('click', () => showView(name));
}

$('ask-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const question = $('ask-input').value.trim();
  if (!question) return;
  const button = $('ask-button');
  const answer = $('answer');
  button.disabled = true;
  button.textContent = 'Thinking…';
  answer.innerHTML = '';
  try {
    const view = await invoke('ask_assistant', {
      question,
      source: $('filter-source').value || null,
      limit: 8,
    });
    renderAnswer(view);
  } catch (err) {
    const p = document.createElement('p');
    p.className = 'no-results';
    p.textContent = String(err);
    answer.appendChild(p);
  } finally {
    button.disabled = false;
    button.textContent = 'Ask';
  }
});

function renderAnswer(view) {
  const answer = $('answer');
  answer.innerHTML = '';

  const card = document.createElement('article');
  card.className = 'answer-card' + (view.local ? '' : ' cloud');

  const text = document.createElement('div');
  text.className = 'answer-text';
  text.textContent = view.text;
  card.appendChild(text);

  const meta = document.createElement('div');
  meta.className = 'hit-prov';
  const badge = document.createElement('span');
  badge.className = 'tag';
  badge.textContent = `${view.adapter}${view.model ? ' · ' + view.model : ''}${view.local ? ' · local' : ' · CLOUD'}`;
  meta.appendChild(badge);
  card.appendChild(meta);
  answer.appendChild(card);

  if (view.sources.length) {
    const label = document.createElement('p');
    label.className = 'result-count';
    label.textContent = 'Sources';
    answer.appendChild(label);
    const list = document.createElement('ol');
    list.className = 'answer-sources';
    for (const s of view.sources) {
      const li = document.createElement('li');
      li.textContent = `${s.title} — ${s.source_name} / ${s.file}`;
      list.appendChild(li);
    }
    answer.appendChild(list);
  }
}

// ---------- Draft ----------

let contractTypes = []; // [{contract_type, precedents, required}] — from the corpus
let lastPreviewOk = false;

const slug = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');

async function refreshContractTypes() {
  contractTypes = await invoke('draft_contract_types');
  const sel = $('draft-type');
  sel.innerHTML = '';
  for (const ct of contractTypes) {
    const o = document.createElement('option');
    o.value = ct.contract_type;
    o.textContent = `${ct.contract_type} (${ct.precedents} precedent${ct.precedents === 1 ? '' : 's'})`;
    sel.appendChild(o);
  }
  if (!contractTypes.length) {
    const o = document.createElement('option');
    o.value = '';
    o.textContent = 'No precedents indexed — attach a source first';
    o.disabled = true;
    sel.appendChild(o);
  }
  const custom = document.createElement('option');
  custom.value = '__custom__';
  custom.textContent = 'Custom…';
  sel.appendChild(custom);
  updateRequiredHint();
}

function updateRequiredHint() {
  const sel = $('draft-type');
  const isCustom = sel.value === '__custom__';
  $('draft-custom-field').hidden = !isCustom;
  const info = contractTypes.find((c) => c.contract_type === sel.value);
  const hint = $('draft-required');
  if (info && info.required.length) {
    hint.textContent =
      'Structure comes from the best-matching precedent. Checklist for this type: ' +
      info.required.join(', ');
  } else if (info) {
    hint.textContent = 'Structure comes from the best-matching precedent. No checklist for this type.';
  } else {
    hint.textContent = isCustom
      ? 'Structure comes from the best-matching precedent in your sources.'
      : '';
  }
}

function addPartyRow(role = '') {
  const row = document.createElement('div');
  row.className = 'party-row';
  const name = document.createElement('input');
  name.className = 'party-name'; name.type = 'text'; name.placeholder = 'Name';
  name.autocomplete = 'off'; name.spellcheck = false;
  const roleInput = document.createElement('input');
  roleInput.className = 'party-role'; roleInput.type = 'text'; roleInput.placeholder = 'Role (e.g. Seller)';
  roleInput.autocomplete = 'off'; roleInput.spellcheck = false; roleInput.value = role;
  const reg = document.createElement('input');
  reg.className = 'party-reg'; reg.type = 'text'; reg.placeholder = 'Reg no (optional)';
  reg.autocomplete = 'off'; reg.spellcheck = false;
  const del = document.createElement('button');
  del.type = 'button'; del.className = 'party-del ghost small'; del.textContent = '×'; del.title = 'Remove party';
  del.addEventListener('click', () => row.remove());
  row.append(name, roleInput, reg, del);
  $('draft-parties').appendChild(row);
}

// Rebuild the "draw from" scope checkboxes, preserving what's ticked.
function renderDraftScope(sources) {
  const box = $('draft-scope');
  if (!box) return;
  const ticked = new Set([...box.querySelectorAll('input:checked')].map((c) => c.value));
  box.innerHTML = '';
  const attached = sources.filter((s) => s.attached);
  if (!attached.length) {
    const span = document.createElement('span');
    span.className = 'hint';
    span.textContent = 'No attached sources yet — attach one to draft from it.';
    box.appendChild(span);
    return;
  }
  for (const s of attached) {
    const label = document.createElement('label');
    label.className = 'scope-item';
    const cb = document.createElement('input');
    cb.type = 'checkbox'; cb.value = s.name; cb.checked = ticked.has(s.name);
    const span = document.createElement('span');
    span.textContent = s.name;
    label.append(cb, span);
    box.appendChild(label);
  }
}

// Read the whole form into a MatterSpec (serde fills the rest with defaults).
function buildSpec() {
  const sel = $('draft-type').value;
  const contract_type = sel === '__custom__' ? $('draft-custom').value.trim() : sel;

  const parties = [...$('draft-parties').querySelectorAll('.party-row')]
    .map((row) => ({
      name: row.querySelector('.party-name').value.trim(),
      role: row.querySelector('.party-role').value.trim(),
      reg_no: row.querySelector('.party-reg').value.trim() || null,
    }))
    .filter((p) => p.name || p.role);
  parties.forEach((p, i) => { p.id = slug(p.role) || `party-${i + 1}`; });

  const source_scope = [...$('draft-scope').querySelectorAll('input:checked')].map((c) => c.value);

  const variables = {};
  for (const inp of $('draft-vars').querySelectorAll('input')) {
    if (inp.value.trim()) variables[inp.dataset.name] = inp.value.trim();
  }

  return {
    contract_type,
    title: $('draft-title').value.trim(),
    jurisdiction: $('draft-jurisdiction').value.trim() || null,
    language: 'English',
    parties,
    variables,
    source_scope,
  };
}

// Variable inputs are rebuilt after each preview; keep what the user has typed.
function renderVars(vars) {
  const box = $('draft-vars');
  const current = {};
  for (const inp of box.querySelectorAll('input')) current[inp.dataset.name] = inp.value;
  box.innerHTML = '';
  $('draft-vars-field').hidden = vars.length === 0;
  for (const v of vars) {
    const wrap = document.createElement('label');
    wrap.className = 'field var-field' + (v.filled ? '' : ' unfilled');
    const span = document.createElement('span');
    span.textContent = v.label || v.name;
    const inp = document.createElement('input');
    inp.type = 'text';
    inp.dataset.name = v.name;
    inp.value = current[v.name] || '';
    inp.placeholder = v.filled ? '' : 'required';
    inp.autocomplete = 'off';
    wrap.append(span, inp);
    box.appendChild(wrap);
  }
}

function findingLine(f, kind) {
  const d = document.createElement('div');
  d.className = 'finding ' + kind;
  d.textContent = `[${f.code}] ${f.location}: ${f.message}`;
  return d;
}

function renderDraftClause(c) {
  const card = document.createElement('article');
  card.className = 'hit';
  card.tabIndex = 0;
  // Indent sub-clauses so the preview shows the document's real structure.
  if (c.depth) {
    card.style.marginLeft = Math.min(c.depth, 3) * 1.5 + 'rem';
    card.classList.add('sub-clause');
  }
  const head = document.createElement('div');
  head.className = 'hit-head';
  if (c.number) {
    const n = document.createElement('span');
    n.className = 'hit-number';
    n.textContent = c.number;
    head.appendChild(n);
  }
  const t = document.createElement('h3');
  t.className = 'hit-title';
  // Sub-clauses legitimately have no heading; only a top-level one is untitled.
  t.textContent = c.heading || (c.depth ? '' : '(untitled)');
  head.appendChild(t);
  if (c.adapted) {
    const k = document.createElement('span');
    k.className = 'hit-kind';
    k.textContent = 'adapted';
    head.appendChild(k);
  }
  const body = document.createElement('div');
  body.className = 'hit-body';
  body.textContent = c.snippet;
  const prov = document.createElement('div');
  prov.className = 'hit-prov';
  const tag = document.createElement('span');
  tag.className = 'tag';
  tag.textContent = c.source ? (c.source + (c.file ? ' / ' + c.file : '')) : 'assembled';
  prov.appendChild(tag);
  card.append(head, body, prov);
  const toggle = () => card.classList.toggle('open');
  card.addEventListener('click', toggle);
  card.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggle(); }
  });
  return card;
}

function renderDraftPreview(v) {
  const out = $('draft-output');
  out.innerHTML = '';

  const banner = document.createElement('div');
  banner.className = 'draft-status ' + (v.renderable ? 'ok' : 'blocked');
  banner.textContent = v.renderable
    ? `Ready — ${v.clauses.length} clause${v.clauses.length === 1 ? '' : 's'} assembled, no blocking errors.`
    : `${v.errors.length} error${v.errors.length === 1 ? '' : 's'} block rendering — resolve them, then preview again.`;
  out.appendChild(banner);

  const rep = document.createElement('div');
  rep.className = 'draft-report';
  const repLine = (cls, text) => {
    const line = document.createElement('div');
    line.className = 'rep-line ' + cls;
    line.textContent = text;
    rep.appendChild(line);
  };

  // Which precedent gave the draft its structure — the single most important
  // thing for a drafter to be able to check.
  if (v.skeleton) {
    const s = v.skeleton;
    repLine(
      s.exact_type_match ? 'ok' : 'warn',
      `Structure from ${s.source} / ${s.file} — ${s.clause_count} clauses` +
        (s.exact_type_match ? '' : ` (a ${s.contract_type || 'document of another type'})`)
    );
  }
  if (v.schedules.length) repLine('ok', `Schedules carried through: ${v.schedules.join(', ')}`);
  if (v.definitions.length) repLine('ok', `${v.definitions.length} defined terms carried through`);
  for (const f of v.gap_filled) {
    repLine('ok', `+ ${f.clause_type} ← ${f.source}${f.file ? ' / ' + f.file : ''} (not in the precedent)`);
  }
  for (const e of v.excluded) repLine('warn', `− ${e} — excluded by this matter`);
  for (const m of v.missing) repLine('miss', `✗ ${m} — required, and no precedent found anywhere`);
  for (const n of v.notes) repLine('warn', `! ${n}`);
  if (rep.childElementCount) out.appendChild(rep);

  if (v.errors.length || v.warnings.length) {
    const val = document.createElement('div');
    val.className = 'draft-findings';
    for (const f of v.errors) val.appendChild(findingLine(f, 'error'));
    for (const f of v.warnings) val.appendChild(findingLine(f, 'warning'));
    out.appendChild(val);
  }

  const count = document.createElement('p');
  count.className = 'result-count';
  count.textContent = 'Assembled document';
  out.appendChild(count);
  for (const c of v.clauses) out.appendChild(renderDraftClause(c));
}

const DRAFT_WELCOME =
  '<div class="welcome">' +
  '<p class="welcome-lede">Assemble a contract from your precedents.</p>' +
  '<p>Choose a contract type, name the parties, and preview. DraftOS picks the ' +
  'best-matching precedent in your attached sources and follows <em>its</em> structure — ' +
  'its clauses, sub-clauses, definitions and schedules — renumbering throughout and ' +
  "telling you exactly what's missing or unresolved before it renders.</p>" +
  '</div>';

function resetDraftOutput() {
  $('draft-output').innerHTML = DRAFT_WELCOME;
  $('draft-vars').innerHTML = '';
  $('draft-vars-field').hidden = true;
  lastPreviewOk = false;
  $('draft-save-btn').disabled = true;
}

$('draft-type').addEventListener('change', updateRequiredHint);
$('draft-add-party').addEventListener('click', () => addPartyRow());
$('draft-clear-btn').addEventListener('click', resetDraftOutput);

$('draft-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const spec = buildSpec();
  if (!spec.contract_type) { status('Choose or type a contract type first', true); return; }
  const btn = $('draft-preview-btn');
  btn.disabled = true;
  btn.textContent = 'Assembling…';
  try {
    const view = await invoke('draft_preview', { spec });
    renderDraftPreview(view);
    renderVars(view.variables);
    lastPreviewOk = view.renderable;
    $('draft-save-btn').disabled = !view.renderable;
  } catch (err) {
    const out = $('draft-output');
    out.innerHTML = '';
    const p = document.createElement('p');
    p.className = 'no-results';
    p.textContent = String(err);
    out.appendChild(p);
    lastPreviewOk = false;
    $('draft-save-btn').disabled = true;
  } finally {
    btn.disabled = false;
    btn.textContent = 'Preview draft';
  }
});

$('draft-save-btn').addEventListener('click', async () => {
  const spec = buildSpec();
  const base = (spec.title || spec.contract_type || 'draft').replace(/[^\w.-]+/g, '_');
  let path;
  try {
    path = await invoke('pick_save_path', { defaultName: base + '.docx' });
  } catch (err) {
    status(String(err), true);
    return;
  }
  if (!path) return; // cancelled
  const btn = $('draft-save-btn');
  btn.disabled = true;
  btn.textContent = 'Rendering…';
  try {
    const res = await invoke('draft_save', { spec, path, force: false });
    status(`Saved ${res.clauses}-clause ${res.format.toUpperCase()} → ${res.path}`);
  } catch (err) {
    status(String(err), true);
  } finally {
    btn.disabled = false;
    btn.textContent = 'Save DOCX…';
    $('draft-save-btn').disabled = !lastPreviewOk;
  }
});

// ---------- Events from the Rust side ----------

listen('source-updated', ({ payload }) => {
  if (payload.indexed > 0 || payload.removed > 0) {
    status(`${payload.name}: indexed ${payload.indexed} file(s), removed ${payload.removed}`);
  }
  refreshSources();
});
listen('source-error', ({ payload }) => {
  status(`${payload.name}: ${payload.message}`, true);
});

// ---------- Init ----------

const typeFilter = $('filter-type');
for (const t of CLAUSE_TYPES) {
  const opt = document.createElement('option');
  opt.value = t;
  opt.textContent = t;
  typeFilter.appendChild(opt);
}

refreshSources().catch((err) => status(String(err), true));
refreshModel().catch((err) => status(String(err), true));
refreshContractTypes().catch((err) => status(String(err), true));
addPartyRow();
