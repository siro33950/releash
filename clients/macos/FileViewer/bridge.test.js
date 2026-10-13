import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import { build } from 'esbuild';

class Element {
  constructor(tagName) { this.tagName = tagName; }
  children = [];
  attributes = {};
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute(name, value) { this.attributes[name] = value; }
}

async function bridge() {
  const instances = [];
  const messages = [];
  class Renderer {
    constructor(options) { this.options = options; instances.push(this); }
    setOptions(options) { this.options = options; }
    render(input) { this.input = input; }
    setSelectedLines() {}
    cleanUp() { this.cleaned = true; }
  }
  const elements = Object.fromEntries(['viewport', 'content', 'file'].map((id) => [id, new Element()]));
  const window = {
    testDiffs: {
      VirtualizedFile: class extends Renderer {},
      VirtualizedFileDiff: class extends Renderer {},
      Virtualizer: class { setup() {} },
    },
    webkit: { messageHandlers: { fileViewer: { postMessage: (value) => messages.push(value) } } },
  };
  const source = await readFile(new URL('./bridge.js', import.meta.url), 'utf8');
  const compiled = await build({
    stdin: {
      contents: source.replace("import { VirtualizedFile, VirtualizedFileDiff, Virtualizer } from '@pierre/diffs';",
        'const { VirtualizedFile, VirtualizedFileDiff, Virtualizer } = window.testDiffs;'),
      resolveDir: new URL('.', import.meta.url).pathname,
    },
    bundle: true, write: false, format: 'iife',
  });
  vm.runInNewContext(compiled.outputFiles[0].text, {
    window, document: { getElementById: (id) => elements[id], createElement: (name) => new Element(name) },
    TextEncoder, TextDecoder, console,
  });
  return { window, messages, instances, renderer: () => instances.at(-1) };
}

const payload = () => ({
  path: 'file.rs', version: '1', original: 'a\nb\nc\n', modified: 'a\nc\n',
  patch: '--- a/file.rs\n+++ b/file.rs\n@@ -1,3 +1,2 @@\n a\n-b\n c\n',
  mode: 'diff', base: 'head', style: 'split', full: false, dark: false, deleted: false,
  hunks: [{ hunkId: 'h1', oldStart: 1, oldLines: 3, newStart: 1, newLines: 2, isStaged: false }],
  threads: [],
});
const plain = (value) => JSON.parse(JSON.stringify(value));

test('display choices and system theme reach the renderer, including File and Diff transitions', async () => {
  const ui = await bridge();
  const next = payload();
  ui.window.releashRender(next);
  for (const [style, full, dark] of [['unified', true, true], ['split', false, false]]) {
    ui.window.releashRender({ ...next, style, full, dark });
    assert.equal(ui.renderer().options.diffStyle, style);
    assert.equal(ui.renderer().options.expandUnchanged, full);
    assert.equal(ui.renderer().options.themeType, dark ? 'dark' : 'light');
    assert.equal(ui.renderer().input.fileDiff.deletionLines.join(''), next.original);
  }
  assert.equal(ui.renderer().input.fileContainer.tagName, 'diffs-container');
  const diff = ui.renderer();
  ui.window.releashRender({ ...next, mode: 'file' });
  assert.equal(diff.cleaned, true);
  assert.equal(ui.renderer().input.file.contents, next.modified);
  assert.equal(ui.renderer().options.enableGutterUtility, false);
  assert.equal(ui.renderer().input.lineAnnotations.length, 0);
  ui.window.releashRender(next);
  assert.equal(ui.renderer().input.fileDiff.deletionLines.join(''), next.original);
});

test('creating a thread preserves the selected side and line, then restores it in the annotation', async () => {
  const ui = await bridge();
  const next = payload();
  ui.window.releashRender(next);
  for (const side of ['deletions', 'additions']) {
    const controls = ui.renderer().options.renderGutterUtility(() => ({ side, lineNumber: 2 }));
    controls.children.at(-1).onclick();
    assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'create', line: 2, endLine: 2, side, endSide: side });
    ui.window.releashRender({ ...next, threads: [{ id: side, line: 2, side, comments: [] }] });
    const annotation = ui.renderer().input.lineAnnotations[0];
    assert.equal(annotation.side, side);
    assert.equal(annotation.lineNumber, 2);
  }
  ui.renderer().options.onLineSelected({ start: 4, end: 2, side: 'deletions', endSide: 'additions' });
  ui.renderer().options.renderGutterUtility(() => ({ side: 'deletions', lineNumber: 4 })).children.at(-1).onclick();
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'create', line: 2, endLine: 4, side: 'additions', endSide: 'deletions' });
});

test('threads show comments, collapse resolved threads and send reply and resolve actions', async () => {
  const ui = await bridge();
  const thread = { id: 't1', line: 2, side: 'deletions', resolved: false, canResolve: true,
    comments: [{ author: 'human', content: 'selected deleted line' }] };
  ui.window.releashRender({ ...payload(), threads: [thread] });
  const render = ui.renderer().options.renderAnnotation;
  const open = render({ metadata: thread });
  assert.equal(open.open, true);
  assert.equal(open.children[1].textContent, 'human: selected deleted line');
  const input = open.children.find((element) => element.attributes['aria-label'] === 'Threadへの返信');
  input.value = 'reply body';
  open.children.find((element) => element.textContent === '返信').onclick();
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'reply', id: 't1', content: 'reply body', requestID: 1 });
  const replied = { ...thread, comments: [...thread.comments, { author: 'human', content: 'reply body' }] };
  ui.window.releashRender({ ...payload(), threads: [replied] });
  const refreshed = render(ui.renderer().input.lineAnnotations[0]);
  assert.equal(refreshed.children[2].textContent, 'human: reply body');
  open.children.find((element) => element.textContent === '解決').onclick();
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'resolve', id: 't1' });
  ui.window.releashRender({ ...payload(), threads: [{ ...thread, resolved: true }] });
  const resolved = render(ui.renderer().input.lineAnnotations[0]);
  assert.equal(resolved.open, false);
  assert.equal(resolved.children[0].textContent, '解決済み Thread');
  assert.equal(resolved.children.length, 2);
  assert.equal(render({ metadata: { ...thread, canResolve: false } }).children.some((e) => e.textContent === '解決'), false);
});

test('HEAD gutter sends stage and unstage for the hovered hunk, while base comparison hides the action', async () => {
  const ui = await bridge();
  const next = payload();
  for (const staged of [false, true]) {
    ui.window.releashRender({ ...next, hunks: [{ ...next.hunks[0], isStaged: staged }] });
    const controls = ui.renderer().options.renderGutterUtility(() => ({ side: 'deletions', lineNumber: 2 }));
    controls.children[0].onclick();
    assert.deepEqual(plain(ui.messages.at(-1)), { kind: staged ? 'unstage' : 'stage', id: 'h1' });
  }
  ui.window.releashRender({ ...next, base: 'branch-base', hunks: [{ ...next.hunks[0], isStaged: undefined }] });
  assert.equal(ui.renderer().options.renderGutterUtility(() => null).children.length, 1);
  assert.equal(ui.renderer().input.lineAnnotations.length, 0);
});


test('changing comparisons recreates gutter controls in both directions', async () => {
  const ui = await bridge();
  for (const base of ['head', 'branch-base', 'head']) {
    const previous = ui.renderer();
    ui.window.releashRender({ ...payload(), base });
    if (previous) {
      assert.equal(previous.cleaned, true);
      assert.notEqual(ui.renderer(), previous);
    }
    assert.equal(ui.renderer().options.renderGutterUtility(() => null).children.length, base === 'head' ? 2 : 1);
  }
});

test('reply drafts survive subscription updates and failed sends, and clear only after acceptance', async () => {
  const ui = await bridge();
  const thread = { id: 't1', line: 2, comments: [], resolved: false };
  const render = () => ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
  const input = (element) => element.children.find((child) => child.attributes['aria-label'] === 'Threadへの返信');
  ui.window.releashRender({ ...payload(), threads: [thread] });
  const original = input(render());
  original.value = 'unsent reply';
  original.oninput();
  ui.window.releashRender({ ...payload(), threads: [{ ...thread }] });
  const refreshed = render();
  assert.equal(input(refreshed).value, 'unsent reply');
  refreshed.children.find((child) => child.textContent === '返信').onclick();
  const request = ui.messages.at(-1);
  ui.window.releashRender({ ...payload(), threads: [{ ...thread }] });
  assert.equal(input(render()).value, 'unsent reply');
  ui.window.releashReplyAccepted(request.id, request.requestID);
  assert.equal(input(render()).value, '');
  ui.window.releashRender({ ...payload(), threads: [{ ...thread }] });
  assert.equal(input(render()).value, '');
});


test('accepting a reply preserves text edited while the request was pending', async () => {
  const ui = await bridge();
  const thread = { id: 't1', line: 2, comments: [], resolved: false };
  ui.window.releashRender({ ...payload(), threads: [thread] });
  const annotation = () => ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
  const element = annotation();
  const input = element.children.find((child) => child.tagName === 'textarea');
  input.value = 'sent'; input.oninput();
  element.children.find((child) => child.textContent === '返信').onclick();
  const request = ui.messages.at(-1);
  input.value = 'new draft'; input.oninput();
  ui.window.releashReplyAccepted(request.id, request.requestID);
  assert.equal(input.value, 'new draft');
  ui.window.releashRender({ ...payload(), threads: [{ ...thread }] });
  assert.equal(annotation().children.find((child) => child.tagName === 'textarea').value, 'new draft');
});
