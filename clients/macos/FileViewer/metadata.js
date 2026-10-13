import { parsePatchFiles, hydratePartialDiff } from '@pierre/diffs';

export function metadata(payload) {
  const diff = parsePatchFiles(payload.patch, undefined, true)[0]?.files[0];
  if (!diff) throw new Error('Server patch contains no file');
  return hydratePartialDiff('merge', diff, {
    oldFile: { name: payload.path, contents: payload.original },
    newFile: { name: payload.path, contents: payload.modified },
  });
}

export function renderType(payload) {
  return payload.mode === 'diff' && payload.patch ? 'diff' : 'file';
}
