import { VirtualizedFile, VirtualizedFileDiff, Virtualizer } from '@pierre/diffs';
import MarkdownIt from 'markdown-it';
import { metadata, renderType } from './metadata.js';

const viewport = document.getElementById('viewport');
const content = document.getElementById('content');
let container = document.getElementById('file');
const virtualizer = new Virtualizer();
virtualizer.setup(viewport, content);
let renderer;
let mode;
let payload;
let selected;
let diffMetadata;
const replies = new Map();
const pendingReplies = new Map();
const replyInputs = new Map();
let replyVersion = 0;
let composerContent = '';
const expanded = new Set();
const markdown = new MarkdownIt({ html: false });
const relativeTime = (timestamp) => {
  const seconds = Math.max(0, Math.floor(Date.now() / 1000 - timestamp));
  for (const [unit, size] of [['年', 31536000], ['日', 86400], ['時間', 3600], ['分', 60]]) {
    if (seconds >= size) return `${Math.floor(seconds / size)} ${unit}前`;
  }
  return 'たった今';
};
window.releashReplyAccepted = (id, version, accepted = true) => {
  if (pendingReplies.get(id) === version) pendingReplies.delete(id);
  if (!accepted) return;
  if (replies.get(id)?.version !== version) return;
  replies.delete(id);
  const input = replyInputs.get(id);
  if (input) input.value = '';
};
const send = (action) => window.webkit.messageHandlers.fileViewer.postMessage(action);
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
  queueMicrotask(() => input.focus());
  return element;
}
function annotation({ metadata: thread }) {
  if (thread.composer) return composer(thread);
  const element = document.createElement('section');
  element.className = 'thread-card';
  const comments = document.createElement('div');
  const showComments = () => comments.replaceChildren(...thread.comments.map(commentView));
  if (thread.resolved) {
    const summary = document.createElement('header');
    const label = document.createElement('span');
    label.textContent = `${thread.resolvedBy} が解決にしました · ${relativeTime(thread.resolvedAt)}`;
    const toggle = button(expanded.has(thread.id) ? '隠す' : '表示', () => {
      if (expanded.delete(thread.id)) {
        comments.replaceChildren();
        toggle.textContent = '表示';
      } else {
        expanded.add(thread.id);
        showComments();
        toggle.textContent = '隠す';
      }
    });
    summary.append(label, toggle);
    element.append(summary);
    if (expanded.has(thread.id)) showComments();
  } else {
    expanded.delete(thread.id);
    showComments();
  }
  element.append(comments);
  if (!thread.resolved) {
    const input = document.createElement('textarea');
    input.setAttribute('aria-label', 'Threadへの返信');
    input.placeholder = '返信を書く…';
    input.value = replies.get(thread.id)?.content ?? '';
    replyInputs.set(thread.id, input);
    input.oninput = () => replies.set(thread.id, { content: input.value, version: ++replyVersion });
    input.onkeydown = (event) => {
      if (event.key !== 'Enter' || !event.metaKey || event.isComposing || event.repeat) return;
      event.preventDefault();
      if (!input.value.trim() || pendingReplies.has(thread.id)) return;
      if (replies.get(thread.id)?.content !== input.value) input.oninput();
      const draft = replies.get(thread.id);
      pendingReplies.set(thread.id, draft.version);
      send({ kind: 'reply', id: thread.id, content: draft.content, requestID: draft.version });
    };
    element.append(input);
    if (thread.canResolve) element.append(button('解決にする', () => send({ kind: 'resolve', id: thread.id })));
  }
  return element;
}
window.releashRender = (next) => {
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
    diffStyle: next.style,
    expandUnchanged: next.full,
    enableLineSelection: true,
    onLineSelected: selectLines,
    renderAnnotation: annotation,
    unsafeCSS: viewerCSS(next, type),
  };
  if (mode !== type || baseChanged) {
    clearSelection();
    renderer?.cleanUp();
    container = document.createElement('diffs-container');
    container.id = 'file';
    content.replaceChildren(container);
    renderer = type === 'diff' ? new VirtualizedFileDiff(options, virtualizer) : new VirtualizedFile(options, virtualizer);
    mode = type;
  } else renderer.setOptions(options);
  const annotations = [...next.threads];
  if (next.composer) annotations.push({ ...next.composer, composer: true });
  if (mode === 'diff' && next.patch) {
    // 注釈は範囲の最後の行の下に出す
    const lineAnnotations = annotations.map((thread) => ({ side: thread.endSide ?? thread.side ?? (next.deleted ? 'deletions' : 'additions'), lineNumber: thread.endLine ?? thread.line, metadata: thread }));
    renderer.render({ fileContainer: container, fileDiff: diffMetadata ??= metadata(next), lineAnnotations });
  } else {
    renderer.render({ fileContainer: container, file: { name: next.path, contents: next.modified }, lineAnnotations: annotations.map((thread) => ({ lineNumber: thread.endLine ?? thread.line, metadata: thread })) });
  }
};
send({ kind: 'ready' });
