import { VirtualizedFile, VirtualizedFileDiff, Virtualizer } from '@pierre/diffs';
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
const replyInputs = new Map();
let replyVersion = 0;
window.releashReplyAccepted = (id, version) => {
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
function annotation({ metadata: thread }) {
  const element = document.createElement('details');
  element.open = !thread.resolved;
  // 見出しは発言者、本文は発言の内容
  const summary = document.createElement('summary');
  const author = thread.comments[0]?.author ?? '';
  summary.textContent = thread.resolved ? `${author}（解決済み）` : author;
  element.append(summary);
  for (const comment of thread.comments) {
    const body = document.createElement('p');
    body.textContent = comment.content;
    element.append(body);
  }
  if (!thread.resolved) {
    const input = document.createElement('textarea');
    input.setAttribute('aria-label', 'Threadへの返信');
    input.value = replies.get(thread.id)?.content ?? '';
    replyInputs.set(thread.id, input);
    input.oninput = () => replies.set(thread.id, { content: input.value, version: ++replyVersion });
    element.append(input, button('返信', () => {
      if (replies.get(thread.id)?.content !== input.value) input.oninput();
      const draft = replies.get(thread.id);
      send({ kind: 'reply', id: thread.id, content: draft.content, requestID: draft.version });
    }));
    if (thread.canResolve) element.append(button('解決', () => send({ kind: 'resolve', id: thread.id })));
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
  if (mode === 'diff' && next.patch) {
    // 注釈は範囲の最後の行の下に出す
    const lineAnnotations = next.threads.map((thread) => ({ side: thread.endSide ?? thread.side ?? (next.deleted ? 'deletions' : 'additions'), lineNumber: thread.endLine ?? thread.line, metadata: thread }));
    renderer.render({ fileContainer: container, fileDiff: diffMetadata ??= metadata(next), lineAnnotations });
  } else {
    renderer.render({ fileContainer: container, file: { name: next.path, contents: next.modified }, lineAnnotations: next.threads.map((thread) => ({ lineNumber: thread.endLine ?? thread.line, metadata: thread })) });
  }
};
send({ kind: 'ready' });
