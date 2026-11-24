import circle_dot from 'lucide-static/icons/circle-dot.svg';
import circle_check from 'lucide-static/icons/circle-check.svg';
import chevron_right from 'lucide-static/icons/chevron-right.svg';
import chevron_down from 'lucide-static/icons/chevron-down.svg';
import { CodeView } from '@pierre/diffs';
import MarkdownIt from 'markdown-it';
import { metadata, renderType } from './metadata.js';

const viewport = document.getElementById('viewport');
let version = 0;
let renderer;
let mode;
let payload;
let selected;
let focusThreadID;
let diffMetadata;
const replies = new Map();
const pendingReplies = new Map();
const replyInputs = new Map();
let nextRequestID = 0;
let composerContent = '';

const markdown = new MarkdownIt({ html: false });
const relativeTime = (timestamp) => {
  const seconds = Math.max(0, Math.floor(Date.now() / 1000 - timestamp));
  for (const [unit, size] of [['年', 31536000], ['日', 86400], ['時間', 3600], ['分', 60]]) {
    if (seconds >= size) return `${Math.floor(seconds / size)} ${unit}前`;
  }
  return 'たった今';
};
window.releashReplyAccepted = (id, requestID, accepted = true) => {
  const pending = pendingReplies.get(id);
  if (!pending || pending.requestID !== requestID) return;
  pendingReplies.delete(id);
  if (!accepted || pending.edited) return;
  replies.delete(id);
  const input = replyInputs.get(id);
  if (input) input.value = '';
};
const send = (action) => window.webkit.messageHandlers.fileViewer.postMessage(action);
let awaitingRestoreRender = false;
function reportScroll() {
  if (awaitingRestoreRender) return;
  const rendered = renderer?.getRenderedItems()[0];
  if (!rendered) return;
  const { instance, item } = rendered;
  const top = renderer.getScrollTop() - renderer.getLocalTopForInstance(instance);
  let line = instance.getNumericScrollAnchor(top);
  if (!line) {
    for (const { lineNumber, side } of item.annotations) {
      const position = instance.getLinePosition(lineNumber, side);
      if (position && position.top < top && top < position.top + position.height &&
          (!line || position.top > line.top)) {
        line = { lineNumber, side, top: position.top };
      }
    }
  }
  send({ kind: 'scroll', anchor: line ? { lineNumber: line.lineNumber, side: line.side, delta: line.top - top } : null });
}
// 行の選択は Swift 側の帯の「Thread作成」が使う。ホバー行へ要素を付け替えるライブラリの gutter は WebKit でスクロール位置を飛ばす（WebKit Bug 286616）ので使わない
const selectLines = (range) => {
  selected = range;
  if (!range) { send({ kind: 'select' }); return; }
  const reversed = range.start > range.end;
  const message = { kind: 'select', line: reversed ? range.end : range.start, endLine: reversed ? range.start : range.end };
  const side = reversed ? range.endSide ?? range.side : range.side;
  const endSide = reversed ? range.side : range.endSide ?? range.side;
  if (side) message.side = side;
  if (endSide) message.endSide = endSide;
  send(message);
};
const clearSelection = () => {
  renderer?.setSelectedLines(null);
  if (selected) selectLines(undefined);
};
const button = (label, action) => {
  const element = document.createElement('button');
  element.textContent = label;
  element.onclick = action;
  return element;
};
// Thread が付いた範囲の行に印を付ける CSS。行は data-line と側で特定する
// 上下: 追加側は data-line が新しい行番号（削除行を除く）。削除側は削除行の data-line か、文脈行の data-alt-line
// 左右: 列ごとに data-additions / data-deletions
const lineSelectors = (line, side, type) => {
  if (type !== 'diff') return [`[data-line="${line}"]`];
  if (side === 'deletions') {
    return [`[data-unified] [data-line-type=change-deletion][data-line="${line}"]`, `[data-unified] [data-alt-line="${line}"]`, `[data-deletions] [data-line="${line}"]`];
  }
  return [`[data-unified] [data-line="${line}"]:not([data-line-type=change-deletion])`, `[data-additions] [data-line="${line}"]`];
};
const threadRangeCSS = (threads, type, accent) => threads.map((thread) => {
  const sides = [...new Set([thread.side, thread.endSide].filter(Boolean))];
  if (!sides.length) sides.push('additions');
  const selectors = [];
  for (let line = thread.line; line <= (thread.endLine ?? thread.line); line++) {
    for (const side of sides) selectors.push(...lineSelectors(line, side, type));
  }
  const tint = thread.resolved ? '' : `background-color:color-mix(in srgb,var(--diffs-bg) 85%,${accent});`;
  return `${selectors.join(',')}{box-shadow:inset 3px 0 ${accent};${tint}}`;
}).join('\n');
// 行選択の色もアクセントカラーに合わせる。ライブラリの既定は独自の青
const viewerCSS = (next, type) => [`:host{--diffs-bg-selection-override:${next.accent};--diffs-bg-selection-number-override:${next.accent}}`, threadRangeCSS(next.threads, type, next.accent)].join('\n');
function commentView(comment) {
  const element = document.createElement('article');
  const heading = document.createElement('header');
  const icon = document.createElement('span');
  icon.className = 'thread-avatar';
  icon.textContent = comment.agent ? '🤖' : Array.from(comment.author)[0] ?? '';
  if (comment.agent) {
    icon.setAttribute('role', 'img');
    icon.setAttribute('aria-label', 'agent');
  }
  const name = document.createElement('span');
  name.textContent = comment.author;
  heading.append(icon, name);
  if (comment.model) {
    const model = document.createElement('span');
    model.textContent = comment.model;
    heading.append(model);
  }
  const time = document.createElement('time');
  time.textContent = relativeTime(comment.createdAt);
  heading.append(time);
  const body = document.createElement('div');
  body.className = 'thread-body';
  body.innerHTML = markdown.render(comment.content);
  element.append(heading, body);
  return element;
}
function composer(thread) {
  const element = document.createElement('section');
  element.className = 'thread-card';
  const input = document.createElement('textarea');
  input.placeholder = 'コメントを書く…（Markdown）';
  input.setAttribute('aria-label', input.placeholder);
  input.value = composerContent;
  input.disabled = thread.sending;
  const create = button('Thread を作る', () => {
    if (input.value.trim()) send({ kind: 'create', content: input.value });
  });
  create.disabled = thread.sending || !input.value.trim();
  input.oninput = () => {
    composerContent = input.value;
    send({ kind: 'editCreate', id: thread.id, content: input.value });
    create.disabled = thread.sending || !input.value.trim();
  };
  const cancel = button('キャンセル', () => send({ kind: 'cancelCreate' }));
  cancel.disabled = thread.sending;
  element.append(input, cancel, create);
  queueMicrotask(() => input.focus({ preventScroll: true }));
  return element;
}
function annotation({ metadata: thread }) {
  if (thread.composer) return composer(thread);
  const element = document.createElement('section');
  element.className = 'thread-card';
  const open = payload.expanded?.includes(thread.id) ?? false;
  const summary = document.createElement('header');
  const icon = (src, name, label) => {
    const image = document.createElement('img');
    image.src = src;
    image.alt = label;
    image.setAttribute('data-icon', name);
    if (label) image.setAttribute('aria-label', label);
    return image;
  };
  const name = document.createElement('span');
  name.textContent = thread.resolved ? thread.resolvedBy : thread.comments[0]?.author;
  const time = document.createElement('time');
  time.textContent = relativeTime(thread.resolved ? thread.resolvedAt : thread.comments.at(-1)?.createdAt);
  const count = document.createElement('span');
  count.textContent = `${thread.comments.length} 件`;
  const toggle = button('', () => {
    focusThreadID = thread.id;
    send({ kind: 'expand', id: thread.id, open: !open });
  });
  if (focusThreadID === thread.id) {
    focusThreadID = undefined;
    queueMicrotask(() => toggle.focus({ preventScroll: true }));
  }
  toggle.setAttribute('aria-label', open ? 'Thread を隠す' : 'Thread を表示');
  toggle.setAttribute('aria-expanded', String(open));
  toggle.append(open ? icon(chevron_down, 'chevron-down', '') : icon(chevron_right, 'chevron-right', ''));
  summary.append(thread.resolved ? icon(circle_check, 'circle-check', '解決済み') : icon(circle_dot, 'circle-dot', '未解決'), name, time, count, toggle);
  element.append(summary);
  if (!open) return element;
  const comments = document.createElement('div');
  comments.append(...thread.comments.map(commentView));
  element.append(comments);
  if (!thread.resolved) {
    const input = document.createElement('textarea');
    input.setAttribute('aria-label', 'Threadへの返信');
    input.placeholder = '返信を書く…';
    input.value = replies.get(thread.id) ?? '';
    replyInputs.set(thread.id, input);
    input.oninput = () => {
      const pending = pendingReplies.get(thread.id);
      if (pending) pending.edited = true;
      replies.set(thread.id, input.value);
      send({ kind: 'editReply', id: thread.id, content: input.value });
    };
    input.onkeydown = (event) => {
      if (event.key !== 'Enter' || !event.metaKey || event.isComposing || event.repeat) return;
      event.preventDefault();
      if (!input.value.trim() || payload.sendingReplies?.includes(thread.id) || pendingReplies.has(thread.id)) return;
      if (replies.get(thread.id) !== input.value) input.oninput();
      const requestID = ++nextRequestID;
      pendingReplies.set(thread.id, { requestID, edited: false });
      send({ kind: 'reply', id: thread.id, content: input.value, requestID });
    };
    element.append(input);
    if (thread.canResolve) element.append(button('解決にする', () => send({ kind: 'resolve', id: thread.id })));
  }
  return element;
}
function expandThreadRegions() {
  if (mode !== 'diff' || payload.full || !payload.threads.length) return;
  const instance = renderer.getRenderedItems()[0]?.instance;
  if (!instance || !diffMetadata) return;
  const { hunks, additionLines } = diffMetadata;
  for (const thread of payload.threads) {
    const deletion = (thread.endSide ?? thread.side ?? (payload.deleted ? 'deletions' : 'additions')) === 'deletions';
    const line = thread.endLine ?? thread.line;
    for (let index = 0; index <= hunks.length; index++) {
      const hunk = hunks[index];
      const additionEnd = (hunk?.additionLineIndex ?? additionLines.length) + 1;
      const end = deletion ? (hunk?.deletionLineIndex ?? diffMetadata.deletionLines.length) + 1 : additionEnd;
      const last = hunks.at(-1);
      const count = hunk?.collapsedBefore ?? (last ? additionLines.length - last.additionLineIndex - last.additionCount : 0);
      if (line >= end) continue;
      if (line >= end - count && !instance.isLineRenderable(additionEnd - (end - line))) {
        instance.expandHunk(index, 'up', count);
      }
      break;
    }
  }
}
window.releashRender = (next) => {
  const firstRender = !payload;
  if (firstRender) {
    awaitingRestoreRender = !!next.scrollAnchor;
    for (const id of next.sendingReplies ?? []) pendingReplies.set(id, { requestID: null, edited: false });
    for (const [id, content] of Object.entries(next.replies ?? {})) {
      replies.set(id, content);
    }
  }
  for (const [id, pending] of pendingReplies) {
    if (pending.requestID !== null || next.sendingReplies?.includes(id)) continue;
    pendingReplies.delete(id);
    if (pending.edited) continue;
    const value = next.replies?.[id] ?? '';
    replies.set(id, value);
    const input = replyInputs.get(id);
    if (input) input.value = value;
  }
  const sourceChanged = !payload || ['path', 'original', 'modified', 'patch'].some((key) => payload[key] !== next[key]);
  if (sourceChanged) {
    clearSelection();
    diffMetadata = undefined;
  }
  const baseChanged = payload?.base !== next.base;
  if (!next.composer || !payload?.composer || next.composer.id !== payload.composer.id) composerContent = next.composer?.content ?? '';
  const openIDs = new Set(next.threads.filter((thread) => !thread.resolved).map((thread) => thread.id));
  for (const id of replyInputs.keys()) if (!openIDs.has(id)) replyInputs.delete(id);
  payload = next;
  const type = renderType(next);
  const options = {
    theme: { dark: 'dark-plus', light: 'light-plus' },
    themeType: next.dark ? 'dark' : 'light',
    disableFileHeader: true,
    layout: { paddingTop: 0, paddingBottom: 0, gap: 0 },
    diffStyle: next.style,
    expandUnchanged: next.full,
    enableLineSelection: true,
    onPostRender: () => queueMicrotask(() => {
      expandThreadRegions();
      awaitingRestoreRender = false;
      reportScroll();
    }),
    onLineSelected: selectLines,
    renderAnnotation: annotation,
    unsafeCSS: viewerCSS(next, type),
  };
  if (mode !== type || baseChanged) {
    clearSelection();
    renderer?.cleanUp();
    renderer = new CodeView(options);
    renderer.setup(viewport);
    renderer.subscribeToScroll(reportScroll);
    mode = type;
  } else renderer.setOptions(options);
  const annotations = [...next.threads];
  if (next.composer) annotations.push({ ...next.composer, composer: true });
  if (mode === 'diff' && next.patch) {
    // 注釈は範囲の最後の行の下に出す
    const lineAnnotations = annotations.map((thread) => ({ side: thread.endSide ?? thread.side ?? (next.deleted ? 'deletions' : 'additions'), lineNumber: thread.endLine ?? thread.line, metadata: thread }));
    renderer.setItems([{ id: next.path, type, version: ++version, fileDiff: diffMetadata ??= metadata(next), annotations: lineAnnotations }]);
  } else {
    renderer.setItems([{ id: next.path, type: 'file', version: ++version, file: { name: next.path, contents: next.modified }, annotations: annotations.map((thread) => ({ lineNumber: thread.endLine ?? thread.line, metadata: thread })) }]);
  }
  if (firstRender && next.scrollAnchor) {
    const { lineNumber, side, delta } = next.scrollAnchor;
    renderer.scrollTo({ type: 'line', id: next.path, lineNumber, side, offset: delta, align: 'start', behavior: 'instant' });
  }
};
send({ kind: 'ready' });
