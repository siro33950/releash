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
const button = (label, action) => {
  const element = document.createElement('button');
  element.textContent = label;
  element.onclick = action;
  return element;
};
function annotation({ metadata: thread }) {
  if (thread.kind === 'hunk') {
    const element = document.createElement('span');
    element.textContent = thread.label;
    return element;
  }
  const element = document.createElement('details');
  element.open = !thread.resolved;
  const summary = document.createElement('summary');
  summary.textContent = thread.resolved ? '解決済み Thread' : 'Thread';
  element.append(summary);
  for (const comment of thread.comments) {
    const body = document.createElement('p');
    body.textContent = `${comment.author}: ${comment.content}`;
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
function gutter(getHovered) {
  const controls = document.createElement('span');
  const stage = button('stage / unstage', () => {
    const hovered = getHovered();
    if (!hovered) return;
    const hunk = payload.hunks.find((h) => {
      const start = hovered.side === 'deletions' ? h.oldStart : h.newStart;
      const count = hovered.side === 'deletions' ? h.oldLines : h.newLines;
      return hovered.lineNumber >= start && hovered.lineNumber < start + Math.max(count, 1);
    });
    if (hunk?.isStaged !== undefined) send({ kind: hunk.isStaged ? 'unstage' : 'stage', id: hunk.hunkId });
  });
  if (payload.base === 'head' && payload.hunks.length) controls.append(stage);
  controls.append(button('Thread作成', () => {
    const row = getHovered();
    if (row) {
      const range = selected ?? { start: row.lineNumber, end: row.lineNumber, side: row.side, endSide: row.side };
      const reversed = range.start > range.end;
      send({ kind: 'create', line: reversed ? range.end : range.start, endLine: reversed ? range.start : range.end,
        side: (reversed ? range.endSide ?? range.side : range.side) ?? row.side,
        endSide: (reversed ? range.side : range.endSide ?? range.side) ?? row.side });
    }
  }));
  return controls;
}
window.releashRender = (next) => {
  const sourceChanged = !payload || ['path', 'original', 'modified', 'patch'].some((key) => payload[key] !== next[key]);
  if (sourceChanged) {
    selected = undefined;
    renderer?.setSelectedLines(null);
    diffMetadata = undefined;
  }
  const baseChanged = payload?.base !== next.base;
  payload = next;
  const options = {
    themeType: next.dark ? 'dark' : 'light',
    disableFileHeader: true,
    diffStyle: next.style,
    expandUnchanged: next.full,
    enableLineSelection: true,
    enableGutterUtility: next.mode === 'diff',
    onLineSelected: (range) => { selected = range; },
    renderAnnotation: annotation,
    renderGutterUtility: next.mode === 'diff' ? (getHovered) => gutter(getHovered) : undefined,
  };
  const type = renderType(next);
  if (mode !== type || baseChanged) {
    renderer?.cleanUp();
    selected = undefined;
    container = document.createElement('diffs-container');
    container.id = 'file';
    content.replaceChildren(container);
    renderer = type === 'diff' ? new VirtualizedFileDiff(options, virtualizer) : new VirtualizedFile(options, virtualizer);
    mode = type;
  } else renderer.setOptions(options);
  if (mode === 'diff' && next.patch) {
    const lineAnnotations = next.threads.map((thread) => ({ side: thread.side ?? (next.deleted ? 'deletions' : 'additions'), lineNumber: thread.line, metadata: thread }));
    for (const hunk of next.hunks) {
      if (hunk.isStaged !== undefined) lineAnnotations.push({ side: hunk.newLines ? 'additions' : 'deletions', lineNumber: hunk.newLines ? hunk.newStart : hunk.oldStart,
        metadata: { kind: 'hunk', label: hunk.isStaged ? 'stage 済み' : '未 stage' } });
    }
    renderer.render({ fileContainer: container, fileDiff: diffMetadata ??= metadata(next), lineAnnotations });
  } else {
    renderer.render({ fileContainer: container, file: { name: next.path, contents: next.modified }, lineAnnotations: next.mode === 'diff' ? next.threads.map((thread) => ({ lineNumber: thread.line, metadata: thread })) : [] });
  }
};
send({ kind: 'ready' });
