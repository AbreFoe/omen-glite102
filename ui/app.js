const T = window.__TAURI__;
const invoke = T.core.invoke;
const appWin = T.window.getCurrentWindow();
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const h = (tag, attrs = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (k === 'class') e.className = v;
    else if (k === 'style') e.style.cssText = v;
    else e.setAttribute(k, v);
  }
  e.append(...kids.flat().filter(k => k != null));
  return e;
};
const icon = id => { const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); s.innerHTML = `<use href="#${id}"/>`; return s; };

let cfg = null, dev = { connected: false, dpi_min: 50, dpi_max: 8000, dpi_step: 50, rates: [1, 2, 4, 8] };
let savedKey = '', selStage = 0, dsState = { connected: false, mute: false, deaf: false };

// ---------- names ----------
const VK = { 1: 'ЛКМ', 2: 'ПКМ', 4: 'СКМ', 5: 'Мышь 4', 6: 'Мышь 5', 8: 'Backspace', 9: 'Tab', 13: 'Enter', 16: 'Shift', 17: 'Ctrl', 18: 'Alt', 19: 'Pause', 20: 'CapsLock', 27: 'Esc', 32: 'Space', 33: 'PgUp', 34: 'PgDn', 35: 'End', 36: 'Home', 37: '←', 38: '↑', 39: '→', 40: '↓', 44: 'PrtSc', 45: 'Ins', 46: 'Del', 91: 'Win', 92: 'Win', 93: 'Menu', 106: 'Num *', 107: 'Num +', 109: 'Num −', 110: 'Num .', 111: 'Num /', 144: 'NumLock', 145: 'ScrollLock', 173: 'Без звука', 174: 'Громкость −', 175: 'Громкость +', 176: 'След. трек', 177: 'Пред. трек', 178: 'Стоп', 179: 'Пауза/Воспр.', 186: ';', 187: '=', 188: ',', 189: '-', 190: '.', 191: '/', 192: '`', 219: '[', 220: '\\', 221: ']', 222: "'" };
for (let i = 0; i < 10; i++) { VK[48 + i] = String(i); VK[96 + i] = 'Num ' + i; }
for (let i = 65; i <= 90; i++) VK[i] = String.fromCharCode(i);
for (let i = 1; i <= 24; i++) VK[111 + i] = 'F' + i;
const vkName = v => VK[v] || `VK ${v}`;
const MEDIA = [[175, 'Громкость +'], [174, 'Громкость −'], [173, 'Без звука'], [179, 'Пауза/Воспр.'], [176, 'След. трек'], [177, 'Пред. трек'], [178, 'Стоп']];

const BTN_NAMES = ['Левая кнопка', 'Правая кнопка', 'Колесо (нажатие)', 'Боковая «Назад»', 'Боковая «Вперёд»', 'Кнопка DPI'];
const MOUSE_BITS = { 1: 'Левый клик', 2: 'Правый клик', 4: 'Средний клик', 8: 'Назад', 16: 'Вперёд' };
const SPECIAL = { 3: 'DPI +', 4: 'DPI −', 5: 'Переключение DPI', 6: 'DPI по умолчанию', 7: 'Снайпер (пока зажата)', 11: 'G-Shift (из G HUB)' };
const ACTIONS = [['keys', 'Сочетание клавиш'], ['media', 'Медиа'], ['text', 'Ввод текста'], ['macro', 'Макрос'], ['launch', 'Запустить'], ['discord_mute', 'Discord: микрофон'], ['discord_deafen', 'Discord: звук'], ['discord_ptt', 'Discord: рация']];

const isMedia = a => a.type === 'keys' && a.keys.length === 1 && MEDIA.some(m => m[0] === a.keys[0]);
function actionLabel(a) {
  switch (a.type) {
    case 'keys': return a.keys.length ? a.keys.map(vkName).join(' + ') : 'Сочетание не задано';
    case 'text': return `Текст «${a.text.slice(0, 24)}${a.text.length > 24 ? '…' : ''}»`;
    case 'macro': return `Макрос · ${a.steps.length} шаг. · ${{ once: 'один раз', while_held: 'пока зажата', toggle: 'вкл/выкл' }[a.repeat || 'once']}`;
    case 'launch': return 'Запуск: ' + (a.target.split(/[\\/]/).pop() || '—');
    case 'discord_mute': return 'Discord: вкл/выкл микрофон';
    case 'discord_deafen': return 'Discord: вкл/выкл звук';
    case 'discord_ptt': return 'Discord: рация (зажал — говоришь)';
    default: return 'Действие не выбрано';
  }
}
function bindingLabel(b) {
  switch (b.kind) {
    case 'mouse': return MOUSE_BITS[b.bit] || `Кнопка мыши 0x${b.bit.toString(16)}`;
    case 'special': return SPECIAL[b.code] || `Особая функция 0x${b.code.toString(16)}`;
    case 'disabled': return 'Отключена';
    case 'app': return actionLabel(b.action);
    default: return 'Своё назначение (из G HUB)';
  }
}

// ---------- state helpers ----------
const mouseKey = c => JSON.stringify([c.dpi_stages, c.default_stage, c.shift_stage, c.report_rate, c.led, c.buttons.map(b => b.kind === 'app' ? 'app' : b)]);
function markDirty() { $('#fab').classList.toggle('show', !!cfg && mouseKey(cfg) !== savedKey); }
let persistT;
function persist() { clearTimeout(persistT); persistT = setTimeout(() => invoke('save_config', { cfg }).catch(snackErr), 250); }

function throttle(fn, ms) {
  let last = 0, t, args;
  return (...a) => {
    args = a; const now = Date.now();
    clearTimeout(t);
    if (now - last >= ms) { last = now; fn(...args); } else t = setTimeout(() => { last = Date.now(); fn(...args); }, ms - (now - last));
  };
}
const liveDpi = throttle(dpi => dev.connected && invoke('live_dpi', { dpi }).catch(() => {}), 60);
const liveLed = throttle(() => dev.connected && invoke('live_led', { led: cfg.led }).catch(() => {}), 120);

let snackT;
function snack(msg, err = false) {
  const s = $('#snack'); s.textContent = msg; s.className = 'snackbar show' + (err ? ' err' : '');
  clearTimeout(snackT); snackT = setTimeout(() => s.classList.remove('show'), err ? 5000 : 2600);
}
const snackErr = e => snack(String(e), true);

// ---------- theme ----------
const darkMQ = matchMedia('(prefers-color-scheme: dark)');
function applyTheme() {
  const mode = cfg.theme_mode === 'system' ? (darkMQ.matches ? 'dark' : 'light') : cfg.theme_mode;
  document.documentElement.dataset.theme = mode;
  const l = cfg.led;
  const ledColor = l.effect === 'off' ? '#888888' : l.effect === 'cycle' ? '#ff4d6d' : l.color;
  const seed = cfg.theme_seed === 'auto' ? (['static', 'breathing'].includes(l.effect) ? l.color : '#7c4dff') : cfg.theme_seed;
  const st = document.documentElement.style;
  st.setProperty('--seed', seed);
  st.setProperty('--led', ledColor);
  for (const id of ['#heroGlow', '#lightGlow']) {
    const art = $(id).parentElement;
    art.className = art.className.replace(/\banim-\S+/g, '').trim() + ' anim-' + l.effect;
    art.style.setProperty('--period', (l.period / 1000) + 's');
  }
}
darkMQ.addEventListener('change', () => cfg && applyTheme());

// ---------- navigation ----------
$$('.rail-item').forEach(b => b.addEventListener('click', () => {
  $$('.rail-item').forEach(x => x.classList.toggle('active', x === b));
  $$('.page').forEach(p => p.classList.toggle('active', p.id === 'page-' + b.dataset.page));
}));
$$('.win').forEach(b => b.addEventListener('click', () => {
  ({ min: () => appWin.minimize(), max: () => appWin.toggleMaximize(), close: () => appWin.close() })[b.dataset.win]();
}));

// ---------- ripple ----------
document.addEventListener('pointerdown', e => {
  const t = e.target.closest('.btn, .chip, .li:not(.locked), .stage, .segmented button, .rail-item .pill, .ds-big, .fab');
  if (!t) return;
  const host = t.classList.contains('rail-item') ? $('.pill', t) : t;
  const r = host.getBoundingClientRect(), d = Math.max(r.width, r.height) * 2;
  const s = h('span', { class: 'ripple', style: `width:${d}px;height:${d}px;left:${e.clientX - r.left - d / 2}px;top:${e.clientY - r.top - d / 2}px` });
  if (getComputedStyle(host).position === 'static') host.style.position = 'relative';
  host.style.overflow = 'hidden';
  host.append(s); setTimeout(() => s.remove(), 550);
});

// ---------- segmented helper ----------
function segmented(el, value, onPick, items) {
  if (items) el.replaceChildren(...items.map(([v, l]) => h('button', { 'data-v': v }, l)));
  $$('button', el).forEach(b => {
    b.classList.toggle('on', b.dataset.v === String(value));
    b.onclick = () => onPick(b.dataset.v);
  });
}

// ---------- slider fill ----------
function fill(sl) { sl.style.setProperty('--p', ((sl.value - sl.min) / (sl.max - sl.min) * 100) + '%'); }

// ---------- MOUSE page ----------
function renderMouse() {
  const st = cfg.dpi_stages;
  selStage = Math.min(selStage, st.length - 1);
  $('#stages').replaceChildren(
    ...st.map((v, i) => h('button', { class: 'stage' + (i === selStage ? ' sel' : ''), onclick: () => { selStage = i; liveDpi(st[i]); renderMouse(); } },
      h('div', { class: 'n' }, `Уровень ${i + 1}`, i === cfg.default_stage ? h('span', { class: 'tag' }, 'старт') : null, i === cfg.shift_stage ? h('span', { class: 'tag' }, 'снайпер') : null),
      h('div', { class: 'v' }, v))),
    st.length < 5 ? h('button', { class: 'stage add', title: 'Добавить уровень', onclick: () => { st.push(Math.min(dev.dpi_max, st[st.length - 1] * 2)); selStage = st.length - 1; changed(); } }, icon('i-plus')) : null
  );
  const sl = $('#dpiSlider'), inp = $('#dpiInput');
  for (const e of [sl, inp]) { e.min = dev.dpi_min; e.max = dev.dpi_max; e.step = dev.dpi_step; }
  sl.value = inp.value = st[selStage];
  fill(sl);
  $('#heroDpi').textContent = st[selStage];
  $('#heroSub').textContent = dev.connected ? `${dev.name} · уровень ${selStage + 1} из ${st.length}` : 'Мышь не подключена';
  $('#delStage').disabled = st.length <= 1;
  $('#setDefault').disabled = cfg.default_stage === selStage;
  $('#setShift').disabled = cfg.shift_stage === selStage;
  segmented($('#rateSeg'), cfg.report_rate, v => { cfg.report_rate = +v; dev.connected && invoke('live_rate', { ms: +v }).catch(snackErr); changed(); },
    [...dev.rates].sort((a, b) => b - a).map(ms => [ms, `${Math.round(1000 / ms)} Гц`]));
}
function setStage(v) {
  const s = dev.dpi_step;
  v = Math.max(dev.dpi_min, Math.min(dev.dpi_max, Math.round(v / s) * s));
  cfg.dpi_stages[selStage] = v; liveDpi(v);
  $('#dpiInput').value = v; $('#heroDpi').textContent = v;
  $$('#stages .stage .v')[selStage].textContent = v;
  fill($('#dpiSlider'));
  markDirty();
}
$('#dpiSlider').addEventListener('input', e => setStage(+e.target.value));
$('#dpiInput').addEventListener('change', e => { setStage(+e.target.value); $('#dpiSlider').value = cfg.dpi_stages[selStage]; fill($('#dpiSlider')); });
$('#setDefault').onclick = () => { cfg.default_stage = selStage; changed(); };
$('#setShift').onclick = () => { cfg.shift_stage = selStage; changed(); };
$('#delStage').onclick = () => {
  cfg.dpi_stages.splice(selStage, 1);
  const fix = i => i === selStage ? 0 : i > selStage ? i - 1 : i;
  cfg.default_stage = fix(cfg.default_stage); cfg.shift_stage = fix(cfg.shift_stage);
  selStage = Math.max(0, selStage - 1); changed();
};

// ---------- LIGHT page ----------
const LED_COLORS = ['#ff3b30', '#ff9500', '#ffcc00', '#34c759', '#00c7be', '#0a84ff', '#5856d6', '#af52de', '#ff2d55', '#ffffff'];
function swatchRow(el, colors, current, onPick, withAuto) {
  const custom = h('label', { class: 'sw custom', title: 'Свой цвет' }, h('input', { type: 'color', value: current.startsWith('#') ? current : '#7c4dff' }));
  $('input', custom).addEventListener('input', e => onPick(e.target.value));
  const known = colors.includes(current.toLowerCase()) || current === 'auto';
  if (!known) custom.classList.add('on');
  el.replaceChildren(
    withAuto ? h('button', { class: 'sw auto' + (current === 'auto' ? ' on' : ''), onclick: () => onPick('auto') }, 'Как подсветка') : null,
    ...colors.map(c => h('button', { class: 'sw' + (c === current.toLowerCase() ? ' on' : ''), style: `--c:${c}`, title: c, onclick: () => onPick(c) })),
    custom);
}
function renderLight() {
  const l = cfg.led;
  segmented($('#effectSeg'), l.effect, v => { l.effect = v; ledChanged(); });
  $('#colorBlock').style.display = ['static', 'breathing'].includes(l.effect) ? '' : 'none';
  $('#speedBlock').style.display = ['breathing', 'cycle'].includes(l.effect) ? '' : 'none';
  $('#brightBlock').style.display = ['breathing', 'cycle'].includes(l.effect) ? '' : 'none';
  swatchRow($('#swatches'), LED_COLORS, l.color, c => { l.color = c; ledChanged(); });
  const sp = $('#speed'), br = $('#bright');
  sp.value = 21000 - l.period; br.value = l.brightness; fill(sp); fill(br);
  $('#speedVal').textContent = (l.period / 1000).toFixed(1) + ' с';
  $('#brightVal').textContent = l.brightness + '%';
}
function ledChanged(rerender = true) { liveLed(); applyTheme(); if (rerender) renderLight(); markDirty(); }
$('#speed').addEventListener('input', e => { cfg.led.period = 21000 - +e.target.value; fill(e.target); $('#speedVal').textContent = (cfg.led.period / 1000).toFixed(1) + ' с'; ledChanged(false); });
$('#bright').addEventListener('input', e => { cfg.led.brightness = +e.target.value; fill(e.target); $('#brightVal').textContent = cfg.led.brightness + '%'; ledChanged(false); });

// ---------- BUTTONS page ----------
function renderButtons() {
  $('#btnList').replaceChildren(...cfg.buttons.map((b, i) => h('button', {
    class: 'li' + (i === 0 ? ' locked' : ''),
    title: i === 0 ? 'Левую кнопку менять нельзя — иначе можно остаться без клика' : '',
    onclick: () => i && openEditor(i),
    onmouseenter: () => $$('.pin').forEach(p => p.classList.toggle('hl', +p.dataset.b === i)),
    onmouseleave: () => $$('.pin').forEach(p => p.classList.remove('hl')),
  },
    h('span', { class: 'num' }, i + 1),
    h('span', {}, h('div', { class: 't' }, BTN_NAMES[i], b.kind === 'app' ? h('span', { class: 'badge' }, 'GLite') : null), h('div', { class: 's' }, bindingLabel(b))),
    i ? icon('i-chev') : null)));
  $$('#btnList .chev, #btnList svg').forEach(s => s.classList.add('chev'));
}
$$('.pin').forEach(p => p.addEventListener('click', () => +p.dataset.b && openEditor(+p.dataset.b)));

// ---------- key recorders ----------
let stopRecorder = null;
function recordCombo(el, onDone) {
  stopRecorder?.();
  const keys = [], down = new Set();
  el.classList.add('rec'); el.replaceChildren('Нажми сочетание…');
  const kd = e => {
    e.preventDefault(); e.stopPropagation();
    if (e.repeat) return;
    if (!keys.includes(e.keyCode)) keys.push(e.keyCode);
    down.add(e.keyCode);
    el.replaceChildren(...keys.map(k => h('span', { class: 'kc' }, vkName(k))));
  };
  const ku = e => { e.preventDefault(); down.delete(e.keyCode); if (!down.size && keys.length) finish(true); };
  const finish = ok => {
    removeEventListener('keydown', kd, true); removeEventListener('keyup', ku, true);
    el.classList.remove('rec'); stopRecorder = null;
    onDone(ok ? keys : null);
  };
  addEventListener('keydown', kd, true); addEventListener('keyup', ku, true);
  stopRecorder = () => finish(false);
}
function comboField(keys, onChange) {
  const el = h('button', { class: 'keycap-field' });
  const show = () => el.replaceChildren(...(keys.length ? keys.map(k => h('span', { class: 'kc' }, vkName(k))) : ['Нажми, чтобы записать']));
  el.onclick = () => recordCombo(el, k => { if (k) { keys = k; onChange(k); } show(); });
  show();
  return el;
}

// ---------- editor dialog ----------
let edIdx = 0, ed = null, macroRec = null;
function openEditor(i) {
  edIdx = i; ed = structuredClone(cfg.buttons[i]);
  $('#dlgTitle').textContent = `${i + 1}. ${BTN_NAMES[i]}`;
  renderEditor();
  $('#scrim').classList.add('show'); $('#dlg').classList.add('show');
}
function closeEditor() {
  stopRecorder?.(); macroRec?.stop();
  $('#scrim').classList.remove('show'); $('#dlg').classList.remove('show');
}
$('#scrim').onclick = closeEditor;
$('#dlgCancel').onclick = closeEditor;
$('#dlgOk').onclick = () => { cfg.buttons[edIdx] = ed; closeEditor(); changed(); persist(); };
$('#dlgTest').onclick = () => {
  if (ed.kind !== 'app') return;
  invoke('test_action', { action: ed.action });
  snack('Действие выполнится через полсекунды');
};

function renderEditor() {
  const opts = [
    ...Object.entries(MOUSE_BITS).map(([bit, l]) => [{ kind: 'mouse', bit: +bit }, l]),
    ...[5, 3, 4, 7].map(code => [{ kind: 'special', code }, SPECIAL[code]]),
    [{ kind: 'app', action: { type: 'none' } }, 'Действие GLite'],
    [{ kind: 'disabled' }, 'Отключить'],
  ];
  if (ed.kind === 'raw' || (ed.kind === 'special' && ![5, 3, 4, 7].includes(ed.code))) opts.push([ed, bindingLabel(ed)]);
  const same = (a, b) => a.kind === b.kind && (a.kind === 'app' || JSON.stringify(a) === JSON.stringify(b));
  $('#bindChips').replaceChildren(...opts.map(([b, l]) => h('button', {
    class: 'chip' + (same(ed, b) ? ' on' : ''),
    onclick: () => { if (!same(ed, b)) { ed = structuredClone(b); renderEditor(); } },
  }, l)));
  $('#actionBox').style.display = ed.kind === 'app' ? '' : 'none';
  $('#dlgTest').disabled = ed.kind !== 'app' || ed.action.type === 'none';
  if (ed.kind !== 'app') return;
  const a = ed.action, cur = isMedia(a) ? 'media' : a.type;
  $('#actChips').replaceChildren(...ACTIONS.map(([t, l]) => h('button', {
    class: 'chip' + (cur === t ? ' on' : ''),
    onclick: () => {
      stopRecorder?.(); macroRec?.stop();
      ed.action = t === 'keys' ? { type: 'keys', keys: [] } : t === 'media' ? { type: 'keys', keys: [179] }
        : t === 'text' ? { type: 'text', text: '' } : t === 'macro' ? { type: 'macro', steps: [], repeat: 'once' }
        : t === 'launch' ? { type: 'launch', target: '' } : { type: t };
      renderEditor();
    },
  }, l)));
  $('#actEditor').replaceChildren(actionEditor(a, cur));
}

function actionEditor(a, cur) {
  switch (cur) {
    case 'keys':
      return h('div', { class: 'kv' }, h('span', {}, 'Сочетание (зажимается вместе с кнопкой)'), comboField(a.keys, k => { a.keys = k; $('#dlgTest').disabled = false; }));
    case 'media': {
      const box = h('div', { class: 'chips' });
      const draw = () => box.replaceChildren(...MEDIA.map(([vk, l]) => h('button', { class: 'chip' + (a.keys[0] === vk ? ' on' : ''), onclick: () => { a.keys = [vk]; draw(); } }, l)));
      draw(); return box;
    }
    case 'text': {
      const ta = h('textarea', { placeholder: ' ' }); ta.value = a.text;
      ta.oninput = () => { a.text = ta.value; $('#dlgTest').disabled = !ta.value; };
      return h('div', { class: 'field' }, ta, h('label', {}, 'Текст, который будет напечатан'));
    }
    case 'launch': {
      const inp = h('input', { placeholder: ' ' }); inp.value = a.target;
      inp.oninput = () => { a.target = inp.value; $('#dlgTest').disabled = !inp.value; };
      return h('div', {}, h('div', { class: 'field' }, inp, h('label', {}, 'Программа, файл или ссылка')),
        h('div', { class: 'hint' }, 'Например: C:\\Program Files\\OBS\\obs64.exe, calc, https://youtube.com'));
    }
    case 'macro': return macroEditor(a);
    case 'discord_mute': return h('p', { class: 'hint block' }, 'Нажатие переключает микрофон в Discord. Способ управления настраивается на вкладке Discord.');
    case 'discord_deafen': return h('p', { class: 'hint block' }, 'Нажатие переключает звук (полное заглушение) в Discord.');
    case 'discord_ptt': return h('p', { class: 'hint block' }, 'Пока кнопка зажата — микрофон включён, отпустил — выключен. В режиме горячих клавиш перед использованием выключи микрофон.');
    default: return h('p', { class: 'hint block' }, 'Выбери, что должна делать кнопка.');
  }
}

function macroEditor(a) {
  const wrap = h('div');
  const seg = h('div', { class: 'segmented compact' }, h('button', { 'data-v': 'once' }, 'Один раз'), h('button', { 'data-v': 'while_held' }, 'Пока зажата'), h('button', { 'data-v': 'toggle' }, 'Вкл/выкл'));
  const drawSeg = () => segmented(seg, a.repeat || 'once', v => { a.repeat = v; drawSeg(); });
  drawSeg();
  const list = h('div', { class: 'macro-steps' });
  const draw = () => {
    $('#dlgTest').disabled = !a.steps.length;
    list.replaceChildren(...(a.steps.length ? a.steps.map((s, i) => {
      const del = h('button', { title: 'Удалить', onclick: () => { a.steps.splice(i, 1); draw(); } }, icon('i-close'));
      if (s.kind === 'delay') {
        const inp = h('input', { type: 'number', min: 0, step: 10 }); inp.value = s.ms;
        inp.onchange = () => { s.ms = Math.max(0, +inp.value || 0); };
        return h('span', { class: 'ms delay' }, '⏱', inp, 'мс', del);
      }
      const arrow = { down: '↓ ', up: '↑ ', tap: '' }[s.kind];
      return h('span', { class: 'ms ' + s.kind }, arrow + vkName(s.vk), del);
    }) : [h('span', { class: 'empty' }, 'Нажми «Запись» и понажимай клавиши — порядок и паузы сохранятся.')]));
    list.scrollTop = list.scrollHeight;
  };
  const recBtn = h('button', { class: 'btn tonal' }, icon('i-rec'), 'Запись');
  recBtn.onclick = () => {
    if (macroRec) return macroRec.stop();
    stopRecorder?.();
    let last = 0;
    a.steps = [];
    const push = (kind, vk) => {
      const now = performance.now();
      if (last) a.steps.push({ kind: 'delay', vk: 0, ms: Math.max(10, Math.round((now - last) / 10) * 10) });
      last = now; a.steps.push({ kind, vk, ms: 0 }); draw();
    };
    const kd = e => { e.preventDefault(); e.stopPropagation(); if (!e.repeat) push('down', e.keyCode); };
    const ku = e => { e.preventDefault(); e.stopPropagation(); push('up', e.keyCode); };
    addEventListener('keydown', kd, true); addEventListener('keyup', ku, true);
    recBtn.replaceChildren(icon('i-close'), 'Стоп'); recBtn.classList.replace('tonal', 'filled');
    macroRec = { stop() { removeEventListener('keydown', kd, true); removeEventListener('keyup', ku, true); macroRec = null; recBtn.replaceChildren(icon('i-rec'), 'Запись'); recBtn.classList.replace('filled', 'tonal'); } };
    draw();
  };
  const add = (label, step) => h('button', { class: 'btn text', onclick: () => { a.steps.push(step()); draw(); } }, icon('i-plus'), label);
  draw();
  wrap.append(
    h('div', { class: 'kv' }, h('span', {}, 'Повтор'), seg),
    list,
    h('div', { class: 'row' }, recBtn, add('Пауза', () => ({ kind: 'delay', vk: 0, ms: 100 })), add('ЛКМ', () => ({ kind: 'tap', vk: 1, ms: 0 })), add('ПКМ', () => ({ kind: 'tap', vk: 2, ms: 0 })),
      h('span', { class: 'grow' }), h('button', { class: 'btn text danger', onclick: () => { a.steps = []; draw(); } }, 'Очистить')));
  return wrap;
}

// ---------- DISCORD page ----------
function renderDiscord() {
  const d = cfg.discord;
  segmented($('#dsModeSeg'), d.mode, v => { d.mode = v; persist(); renderDiscord(); });
  $('#dsHotkey').style.display = d.mode === 'hotkey' ? '' : 'none';
  $('#dsRpc').style.display = d.mode === 'rpc' ? '' : 'none';
  $('#dsMuteKeys').replaceWith(Object.assign(comboField(d.mute_keys, k => { d.mute_keys = k; persist(); }), { id: 'dsMuteKeys' }));
  $('#dsDeafKeys').replaceWith(Object.assign(comboField(d.deafen_keys, k => { d.deafen_keys = k; persist(); }), { id: 'dsDeafKeys' }));
  $('#dsClientId').value = d.client_id;
  $('#dsSecret').value = d.client_secret;
  renderDsState();
}
function renderDsState() {
  const rpc = cfg.discord.mode === 'rpc';
  $('#dsMuteBtn').classList.toggle('active', rpc && dsState.mute);
  $('#dsDeafBtn').classList.toggle('active', rpc && dsState.deaf);
  $('#dsState').textContent = !rpc ? 'Режим горячих клавиш: GLite нажимает сочетания, состояние не отслеживается'
    : dsState.connected ? `Подключено · микрофон ${dsState.mute ? 'выключен' : 'включён'} · звук ${dsState.deaf ? 'выключен' : 'включён'}` : 'Не подключено';
}
$('#dsClientId').oninput = e => { cfg.discord.client_id = e.target.value.trim(); cfg.discord.access_token = cfg.discord.refresh_token = ''; persist(); };
$('#dsSecret').oninput = e => { cfg.discord.client_secret = e.target.value.trim(); persist(); };
$('#dsConnect').onclick = async () => {
  await invoke('save_config', { cfg });
  snack('Подтверди доступ в окне Discord');
  invoke('discord_connect');
};
$('#dsMuteBtn').onclick = () => invoke('discord_action', { what: 'mute' });
$('#dsDeafBtn').onclick = () => invoke('discord_action', { what: 'deafen' });
T.event.listen('discord', ({ payload }) => {
  dsState = payload;
  if (payload.error) snack('Discord: ' + payload.error, true);
  if (cfg) renderDsState();
});

// ---------- SETTINGS page ----------
const SEEDS = ['#7c4dff', '#0b57d0', '#006a6a', '#386a20', '#b3261e', '#8b5000', '#984061'];
function renderSettings() {
  segmented($('#themeSeg'), cfg.theme_mode, v => { cfg.theme_mode = v; applyTheme(); persist(); renderSettings(); });
  swatchRow($('#seedSwatches'), SEEDS, cfg.theme_seed, c => { cfg.theme_seed = c; applyTheme(); persist(); renderSettings(); }, true);
  $('#autostart').checked = cfg.autostart;
  $('#startMin').checked = cfg.start_minimized;
}
$('#autostart').onchange = async e => {
  try { await invoke('set_autostart', { on: e.target.checked }); cfg.autostart = e.target.checked; persist(); }
  catch (err) { e.target.checked = !e.target.checked; snackErr(err); }
};
$('#startMin').onchange = e => { cfg.start_minimized = e.target.checked; persist(); };
$('#readMouse').onclick = () => syncFromMouse(true);
$('#restore').onclick = async () => {
  if (!confirm('Вернуть профиль, который был в мыши до первой записи из GLite?')) return;
  try { await invoke('restore_backup'); await syncFromMouse(false); snack('Исходный профиль восстановлен'); } catch (e) { snackErr(e); }
};

// ---------- write ----------
$('#fab').onclick = async () => {
  const fab = $('#fab');
  if (fab.classList.contains('busy')) return;
  fab.classList.add('busy'); $('span', fab).textContent = 'Записываю…';
  try {
    await invoke('write_mouse', { cfg });
    savedKey = mouseKey(cfg); snack('Настройки записаны в память мыши');
  } catch (e) { snackErr(e); }
  fab.classList.remove('busy'); $('span', fab).textContent = 'Записать в мышь';
  markDirty();
};

// ---------- device ----------
async function refreshDevice() {
  dev = await invoke('get_device');
  $('#devChip').classList.toggle('on', dev.connected);
  $('#devName').textContent = dev.connected ? dev.name : 'Мышь не подключена';
}
async function syncFromMouse(announce) {
  try {
    cfg = await invoke('read_mouse');
    savedKey = mouseKey(cfg);
    selStage = cfg.default_stage;
    renderAll();
    if (announce) snack('Настройки прочитаны из мыши');
  } catch (e) { snackErr(e); }
}
T.event.listen('device', async () => {
  const was = dev.connected;
  await refreshDevice();
  if (dev.connected && !was && cfg && mouseKey(cfg) === savedKey) await syncFromMouse(false);
  else if (cfg) renderMouse();
});

function changed() { renderAll(); }
function renderAll() { applyTheme(); renderMouse(); renderLight(); renderButtons(); renderDiscord(); renderSettings(); markDirty(); }

(async () => {
  cfg = await invoke('get_config');
  savedKey = mouseKey(cfg);
  await refreshDevice();
  if (dev.connected) await syncFromMouse(false);
  else renderAll();
})();
