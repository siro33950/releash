"use client";

import React from "react"

import { useState, useCallback, useMemo, useRef, useEffect } from "react";
import {
  Check,
  MessageSquare,
  Minus,
  AlignJustify,
  SplitSquareHorizontal,
  X,
  ChevronDown,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { EditorTabs, type EditorTab } from "./editor-tabs";

// Types
type DiffMode = "gutter" | "inline" | "split";
type DiffBase = "HEAD" | "HEAD~1" | "HEAD~5" | "staged";
type ReviewStatus = "pending" | "approved" | "change-requested";

interface LineChange {
  lineNumber: number;
  type: "added" | "removed" | "modified";
  originalContent?: string;
}

interface Hunk {
  id: string;
  startLine: number;
  endLine: number;
  changes: LineChange[];
}

interface LineComment {
  id: string;
  lineNumber: number;
  text: string;
  status: ReviewStatus;
  isHunkComment?: boolean;
  hunkId?: string;
}

interface FileData {
  id: string;
  filename: string;
  filepath: string;
  language: string;
  content: string;
  originalContent: string;
  hunks: Hunk[];
}

// Mock files
const mockFiles: FileData[] = [
  {
    id: "1",
    filename: "auth.ts",
    filepath: "src/lib/auth.ts",
    language: "typescript",
    content: `import { hash, compare } from 'bcrypt';
import { SignJWT, jwtVerify } from 'jose';

const JWT_SECRET = new TextEncoder().encode(
  process.env.JWT_SECRET || 'fallback-secret'
);

export async function hashPassword(password: string): Promise<string> {
  return hash(password, 12);
}

export async function verifyPassword(
  password: string,
  hashedPassword: string
): Promise<boolean> {
  return compare(password, hashedPassword);
}

export async function createToken(userId: string): Promise<string> {
  return new SignJWT({ userId })
    .setProtectedHeader({ alg: 'HS256' })
    .setExpirationTime('7d')
    .sign(JWT_SECRET);
}

export async function verifyToken(token: string) {
  try {
    const { payload } = await jwtVerify(token, JWT_SECRET);
    return payload;
  } catch {
    return null;
  }
}`,
    originalContent: `import { hash, compare } from 'bcrypt';
import { SignJWT, jwtVerify } from 'jose';

const JWT_SECRET = new TextEncoder().encode(
  process.env.JWT_SECRET
);

export async function hashPassword(password: string): Promise<string> {
  return hash(password, 10);
}

export async function verifyPassword(
  password: string,
  hashedPassword: string
): Promise<boolean> {
  return compare(password, hashedPassword);
}

export async function createToken(userId: string): Promise<string> {
  return new SignJWT({ userId })
    .setProtectedHeader({ alg: 'HS256' })
    .setExpirationTime('24h')
    .sign(JWT_SECRET);
}`,
    hunks: [
      {
        id: "h1",
        startLine: 5,
        endLine: 5,
        changes: [
          {
            lineNumber: 5,
            type: "modified",
            originalContent: "  process.env.JWT_SECRET",
          },
        ],
      },
      {
        id: "h2",
        startLine: 9,
        endLine: 9,
        changes: [
          {
            lineNumber: 9,
            type: "modified",
            originalContent: "  return hash(password, 10);",
          },
        ],
      },
      {
        id: "h3",
        startLine: 22,
        endLine: 22,
        changes: [
          {
            lineNumber: 22,
            type: "modified",
            originalContent: "    .setExpirationTime('24h')",
          },
        ],
      },
      {
        id: "h4",
        startLine: 26,
        endLine: 33,
        changes: [
          { lineNumber: 26, type: "added" },
          { lineNumber: 27, type: "added" },
          { lineNumber: 28, type: "added" },
          { lineNumber: 29, type: "added" },
          { lineNumber: 30, type: "added" },
          { lineNumber: 31, type: "added" },
          { lineNumber: 32, type: "added" },
          { lineNumber: 33, type: "added" },
        ],
      },
    ],
  },
  {
    id: "2",
    filename: "api.ts",
    filepath: "src/lib/api.ts",
    language: "typescript",
    content: `export async function fetchData<T>(url: string): Promise<T> {
  const response = await fetch(url, {
    headers: {
      'Content-Type': 'application/json',
    },
    cache: 'no-store',
  });
  
  if (!response.ok) {
    throw new Error(\`HTTP error! status: \${response.status}\`);
  }
  
  return response.json();
}`,
    originalContent: `export async function fetchData<T>(url: string): Promise<T> {
  const response = await fetch(url);
  
  if (!response.ok) {
    throw new Error('Failed to fetch');
  }
  
  return response.json();
}`,
    hunks: [
      {
        id: "h5",
        startLine: 2,
        endLine: 7,
        changes: [
          {
            lineNumber: 2,
            type: "modified",
            originalContent: "  const response = await fetch(url);",
          },
          { lineNumber: 3, type: "added" },
          { lineNumber: 4, type: "added" },
          { lineNumber: 5, type: "added" },
          { lineNumber: 6, type: "added" },
          { lineNumber: 7, type: "added" },
        ],
      },
      {
        id: "h6",
        startLine: 10,
        endLine: 10,
        changes: [
          {
            lineNumber: 10,
            type: "modified",
            originalContent: "    throw new Error('Failed to fetch');",
          },
        ],
      },
    ],
  },
];

const baseOptions: { value: DiffBase; label: string }[] = [
  { value: "HEAD", label: "HEAD" },
  { value: "HEAD~1", label: "HEAD~1" },
  { value: "HEAD~5", label: "HEAD~5" },
  { value: "staged", label: "Staged" },
];

export function UnifiedEditor() {
  const [files] = useState<FileData[]>(mockFiles);
  const [openTabs, setOpenTabs] = useState<EditorTab[]>([
    {
      id: "1",
      filename: "auth.ts",
      filepath: "src/lib/auth.ts",
      isDirty: true,
      language: "typescript",
    },
  ]);
  const [activeTabId, setActiveTabId] = useState<string | null>("1");
  const [diffMode, setDiffMode] = useState<DiffMode>("gutter");
  const [diffBase, setDiffBase] = useState<DiffBase>("HEAD");

  // Comments by file -> line
  const [comments, setComments] = useState<Record<string, LineComment[]>>({});
  const [activeCommentLine, setActiveCommentLine] = useState<{
    fileId: string;
    line: number;
  } | null>(null);

  const activeFile = useMemo(
    () => files.find((f) => f.id === activeTabId),
    [files, activeTabId]
  );

  const handleTabClose = useCallback(
    (tabId: string) => {
      setOpenTabs((prev) => prev.filter((t) => t.id !== tabId));
      if (activeTabId === tabId) {
        const remaining = openTabs.filter((t) => t.id !== tabId);
        setActiveTabId(remaining.length > 0 ? remaining[0].id : null);
      }
    },
    [activeTabId, openTabs]
  );

  const openFile = useCallback(
    (file: FileData) => {
      if (!openTabs.find((t) => t.id === file.id)) {
        setOpenTabs((prev) => [
          ...prev,
          {
            id: file.id,
            filename: file.filename,
            filepath: file.filepath,
            isDirty: file.hunks.length > 0,
            language: file.language,
          },
        ]);
      }
      setActiveTabId(file.id);
    },
    [openTabs]
  );

  const addComment = useCallback(
    (
      fileId: string,
      lineNumber: number,
      text: string,
      status: ReviewStatus,
      hunkId?: string
    ) => {
      const newComment: LineComment = {
        id: `c-${Date.now()}`,
        lineNumber,
        text,
        status,
        isHunkComment: !!hunkId,
        hunkId,
      };
      setComments((prev) => ({
        ...prev,
        [fileId]: [...(prev[fileId] || []), newComment],
      }));
      setActiveCommentLine(null);
    },
    []
  );

  const getFileComments = useCallback(
    (fileId: string) => comments[fileId] || [],
    [comments]
  );

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-border bg-card">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">Base:</span>
            <select
              value={diffBase}
              onChange={(e) => setDiffBase(e.target.value as DiffBase)}
              className="bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
            >
              {baseOptions.map((opt) => (
                <option key={opt.value} value={opt.value}>
                  {opt.label}
                </option>
              ))}
            </select>
          </div>
          <div className="h-4 w-px bg-border" />
          <div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
            <button
              type="button"
              onClick={() => setDiffMode("gutter")}
              className={cn(
                "flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors",
                diffMode === "gutter"
                  ? "bg-background shadow-sm text-foreground"
                  : "text-muted-foreground hover:text-foreground"
              )}
              title="Gutter markers only"
            >
              <Minus className="h-3.5 w-3.5" />
              Gutter
            </button>
            <button
              type="button"
              onClick={() => setDiffMode("inline")}
              className={cn(
                "flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors",
                diffMode === "inline"
                  ? "bg-background shadow-sm text-foreground"
                  : "text-muted-foreground hover:text-foreground"
              )}
              title="Inline diff"
            >
              <AlignJustify className="h-3.5 w-3.5" />
              Inline
            </button>
            <button
              type="button"
              onClick={() => setDiffMode("split")}
              className={cn(
                "flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors",
                diffMode === "split"
                  ? "bg-background shadow-sm text-foreground"
                  : "text-muted-foreground hover:text-foreground"
              )}
              title="Split view"
            >
              <SplitSquareHorizontal className="h-3.5 w-3.5" />
              Split
            </button>
          </div>
        </div>
        {activeFile && (
          <div className="flex items-center gap-2 text-xs">
            <span className="text-muted-foreground">
              {activeFile.hunks.length} hunks
            </span>
            <span className="text-muted-foreground">
              {getFileComments(activeFile.id).length} comments
            </span>
          </div>
        )}
      </div>

      {/* Tabs */}
      <EditorTabs
        tabs={openTabs}
        activeTabId={activeTabId}
        onTabSelect={setActiveTabId}
        onTabClose={handleTabClose}
      />

      {/* Editor */}
      <div className="flex-1 overflow-hidden">
        {activeFile ? (
          diffMode === "split" ? (
            <SplitEditor
              file={activeFile}
              diffBase={diffBase}
              comments={getFileComments(activeFile.id)}
              activeCommentLine={
                activeCommentLine?.fileId === activeFile.id
                  ? activeCommentLine.line
                  : null
              }
              onLineClick={(line) =>
                setActiveCommentLine({ fileId: activeFile.id, line })
              }
              onAddComment={(line, text, status, hunkId) =>
                addComment(activeFile.id, line, text, status, hunkId)
              }
              onCancelComment={() => setActiveCommentLine(null)}
            />
          ) : (
            <SingleEditor
              file={activeFile}
              diffMode={diffMode}
              comments={getFileComments(activeFile.id)}
              activeCommentLine={
                activeCommentLine?.fileId === activeFile.id
                  ? activeCommentLine.line
                  : null
              }
              onLineClick={(line) =>
                setActiveCommentLine({ fileId: activeFile.id, line })
              }
              onAddComment={(line, text, status, hunkId) =>
                addComment(activeFile.id, line, text, status, hunkId)
              }
              onCancelComment={() => setActiveCommentLine(null)}
            />
          )
        ) : (
          <EmptyState files={files} onOpenFile={openFile} />
        )}
      </div>
    </div>
  );
}

// Comment Popover
interface CommentPopoverProps {
  lineNumber: number;
  existingComments: LineComment[];
  isOpen: boolean;
  onClose: () => void;
  onSubmit: (text: string, status: ReviewStatus) => void;
  anchorRef: React.RefObject<HTMLElement | null>;
  hunkId?: string;
}

function CommentPopover({
  lineNumber,
  existingComments,
  isOpen,
  onClose,
  onSubmit,
  anchorRef,
  hunkId,
}: CommentPopoverProps) {
  const [text, setText] = useState("");
  const popoverRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (
        popoverRef.current &&
        !popoverRef.current.contains(e.target as Node) &&
        anchorRef.current &&
        !anchorRef.current.contains(e.target as Node)
      ) {
        onClose();
      }
    }
    if (isOpen) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isOpen, onClose, anchorRef]);

  if (!isOpen) return null;

  const lineComments = existingComments.filter(
    (c) => c.lineNumber === lineNumber
  );

  return (
    <div
      ref={popoverRef}
      className="absolute left-16 z-50 w-80 bg-popover border border-border rounded-lg shadow-xl"
      style={{ top: 0 }}
    >
      <div className="flex items-center justify-between px-3 py-2 border-b border-border">
        <span className="text-xs font-medium">
          Line {lineNumber}
          {hunkId && (
            <span className="ml-2 text-muted-foreground">(Hunk)</span>
          )}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="p-0.5 hover:bg-muted rounded"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Existing comments */}
      {lineComments.length > 0 && (
        <div className="max-h-32 overflow-y-auto border-b border-border">
          {lineComments.map((c) => (
            <div key={c.id} className="px-3 py-2 text-xs border-b border-border/50 last:border-b-0">
              <div className="flex items-center gap-2 mb-1">
                {c.status === "approved" ? (
                  <span className="px-1.5 py-0.5 rounded bg-status-added/20 text-status-added text-[10px]">
                    Approved
                  </span>
                ) : (
                  <span className="px-1.5 py-0.5 rounded bg-status-deleted/20 text-status-deleted text-[10px]">
                    Change Request
                  </span>
                )}
              </div>
              <p className="text-foreground">{c.text}</p>
            </div>
          ))}
        </div>
      )}

      {/* New comment form */}
      <div className="p-3 space-y-2">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Add a comment..."
          className="w-full h-16 px-2 py-1.5 text-xs bg-muted border border-border rounded resize-none focus:outline-none focus:ring-1 focus:ring-primary"
          autoFocus
        />
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={() => {
              onSubmit(text || "LGTM", "approved");
              setText("");
            }}
            className="flex items-center gap-1 px-2 py-1 text-xs bg-status-added/20 text-status-added rounded hover:bg-status-added/30"
          >
            <Check className="h-3 w-3" />
            Approve
          </button>
          <button
            type="button"
            onClick={() => {
              if (text.trim()) {
                onSubmit(text, "change-requested");
                setText("");
              }
            }}
            disabled={!text.trim()}
            className="flex items-center gap-1 px-2 py-1 text-xs bg-status-deleted/20 text-status-deleted rounded hover:bg-status-deleted/30 disabled:opacity-50"
          >
            <MessageSquare className="h-3 w-3" />
            Request Change
          </button>
        </div>
      </div>
    </div>
  );
}

// Single pane editor (gutter / inline mode)
interface SingleEditorProps {
  file: FileData;
  diffMode: "gutter" | "inline";
  comments: LineComment[];
  activeCommentLine: number | null;
  onLineClick: (line: number) => void;
  onAddComment: (
    line: number,
    text: string,
    status: ReviewStatus,
    hunkId?: string
  ) => void;
  onCancelComment: () => void;
}

function SingleEditor({
  file,
  diffMode,
  comments,
  activeCommentLine,
  onLineClick,
  onAddComment,
  onCancelComment,
}: SingleEditorProps) {
  const lines = file.content.split("\n");
  const lineRefs = useRef<Map<number, HTMLDivElement>>(new Map());

  // Build line metadata
  const lineInfo = useMemo(() => {
    const info = new Map<
      number,
      { type: "added" | "modified"; hunk: Hunk; originalContent?: string }
    >();
    for (const hunk of file.hunks) {
      for (const change of hunk.changes) {
        info.set(change.lineNumber, {
          type: change.type === "removed" ? "modified" : change.type,
          hunk,
          originalContent: change.originalContent,
        });
      }
    }
    return info;
  }, [file.hunks]);

  // Comments by line
  const commentsByLine = useMemo(() => {
    const map = new Map<number, LineComment[]>();
    for (const c of comments) {
      if (!map.has(c.lineNumber)) map.set(c.lineNumber, []);
      map.get(c.lineNumber)!.push(c);
    }
    return map;
  }, [comments]);

  const getHunkForLine = useCallback(
    (lineNum: number) => {
      return lineInfo.get(lineNum)?.hunk;
    },
    [lineInfo]
  );

  return (
    <div className="h-full overflow-auto font-mono text-sm">
      <div className="min-w-max">
        {lines.map((line, idx) => {
          const lineNum = idx + 1;
          const info = lineInfo.get(lineNum);
          const lineComments = commentsByLine.get(lineNum) || [];
          const hasComments = lineComments.length > 0;
          const hasApproved = lineComments.some((c) => c.status === "approved");
          const hasChangeRequest = lineComments.some(
            (c) => c.status === "change-requested"
          );

          return (
            <div key={lineNum} className="relative">
              {/* Show removed line in inline mode */}
              {diffMode === "inline" && info?.originalContent && (
                <div className="flex bg-diff-remove-bg/40">
                  <div className="w-10 flex-shrink-0 text-right pr-2 text-muted-foreground/50 select-none border-r border-border">
                    -
                  </div>
                  <div className="w-6 flex-shrink-0" />
                  <div className="w-1 flex-shrink-0 bg-status-deleted" />
                  <div className="flex-1 px-4 text-diff-remove/70 line-through whitespace-pre">
                    {info.originalContent}
                  </div>
                </div>
              )}

              {/* Current line */}
              <div
                ref={(el) => {
                  if (el) lineRefs.current.set(lineNum, el);
                }}
                className={cn(
                  "flex group",
                  diffMode === "inline" && info && "bg-diff-add-bg/30"
                )}
              >
                {/* Line number */}
                <div
                  className={cn(
                    "w-10 flex-shrink-0 text-right pr-2 select-none border-r border-border",
                    info
                      ? "text-muted-foreground"
                      : "text-muted-foreground/60"
                  )}
                >
                  {lineNum}
                </div>

                {/* Comment icon area */}
                <div className="w-6 flex-shrink-0 flex items-center justify-center">
                  {hasComments ? (
                    <button
                      type="button"
                      onClick={() => onLineClick(lineNum)}
                      className={cn(
                        "p-0.5 rounded",
                        hasChangeRequest
                          ? "text-status-deleted"
                          : hasApproved
                            ? "text-status-added"
                            : "text-primary"
                      )}
                    >
                      <MessageSquare className="h-3.5 w-3.5 fill-current" />
                    </button>
                  ) : (
                    <button
                      type="button"
                      onClick={() => onLineClick(lineNum)}
                      className="p-0.5 rounded text-muted-foreground/30 opacity-0 group-hover:opacity-100 hover:text-muted-foreground transition-opacity"
                    >
                      <MessageSquare className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>

                {/* Gutter marker */}
                <div
                  className={cn(
                    "w-1 flex-shrink-0",
                    info?.type === "added" && "bg-status-added",
                    info?.type === "modified" && "bg-status-modified"
                  )}
                />

                {/* Code - editable */}
                <div
                  className={cn(
                    "flex-1 px-4 whitespace-pre",
                    diffMode === "inline" && info && "text-diff-add"
                  )}
                  contentEditable
                  suppressContentEditableWarning
                  spellCheck={false}
                >
                  {line || " "}
                </div>
              </div>

              {/* Comment popover */}
              {activeCommentLine === lineNum && (
                <CommentPopover
                  lineNumber={lineNum}
                  existingComments={comments}
                  isOpen={true}
                  onClose={onCancelComment}
                  onSubmit={(text, status) =>
                    onAddComment(lineNum, text, status, getHunkForLine(lineNum)?.id)
                  }
                  anchorRef={{ current: lineRefs.current.get(lineNum) || null }}
                  hunkId={getHunkForLine(lineNum)?.id}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// Split view editor
interface SplitEditorProps {
  file: FileData;
  diffBase: DiffBase;
  comments: LineComment[];
  activeCommentLine: number | null;
  onLineClick: (line: number) => void;
  onAddComment: (
    line: number,
    text: string,
    status: ReviewStatus,
    hunkId?: string
  ) => void;
  onCancelComment: () => void;
}

function SplitEditor({
  file,
  diffBase,
  comments,
  activeCommentLine,
  onLineClick,
  onAddComment,
  onCancelComment,
}: SplitEditorProps) {
  const originalLines = file.originalContent.split("\n");
  const currentLines = file.content.split("\n");
  const lineRefs = useRef<Map<number, HTMLDivElement>>(new Map());

  const changedOriginal = useMemo(() => {
    const set = new Set<number>();
    for (const hunk of file.hunks) {
      for (const c of hunk.changes) {
        if (c.type === "modified" || c.type === "removed") {
          set.add(c.lineNumber);
        }
      }
    }
    return set;
  }, [file.hunks]);

  const changedCurrent = useMemo(() => {
    const map = new Map<number, Hunk>();
    for (const hunk of file.hunks) {
      for (const c of hunk.changes) {
        if (c.type === "added" || c.type === "modified") {
          map.set(c.lineNumber, hunk);
        }
      }
    }
    return map;
  }, [file.hunks]);

  // Comments by line
  const commentsByLine = useMemo(() => {
    const map = new Map<number, LineComment[]>();
    for (const c of comments) {
      if (!map.has(c.lineNumber)) map.set(c.lineNumber, []);
      map.get(c.lineNumber)!.push(c);
    }
    return map;
  }, [comments]);

  const maxLines = Math.max(originalLines.length, currentLines.length);

  return (
    <div className="h-full overflow-auto font-mono text-sm">
      <div className="flex min-w-max">
        {/* Original side */}
        <div className="flex-1 border-r border-border">
          <div className="sticky top-0 px-3 py-1.5 text-xs text-muted-foreground bg-muted/80 border-b border-border backdrop-blur">
            {diffBase} - {file.filepath}
          </div>
          {originalLines.map((line, idx) => {
            const lineNum = idx + 1;
            const isChanged = changedOriginal.has(lineNum);
            return (
              <div
                key={lineNum}
                className={cn("flex", isChanged && "bg-diff-remove-bg/30")}
              >
                <div
                  className={cn(
                    "w-10 flex-shrink-0 text-right pr-2 select-none border-r border-border",
                    isChanged
                      ? "text-muted-foreground"
                      : "text-muted-foreground/60"
                  )}
                >
                  {lineNum}
                </div>
                <div
                  className={cn(
                    "w-1 flex-shrink-0",
                    isChanged && "bg-status-deleted"
                  )}
                />
                <div
                  className={cn(
                    "flex-1 px-3 whitespace-pre",
                    isChanged && "text-diff-remove"
                  )}
                >
                  {line || " "}
                </div>
              </div>
            );
          })}
          {/* Padding if shorter */}
          {Array.from({ length: maxLines - originalLines.length }).map(
            (_, i) => (
              <div key={`pad-${i}`} className="flex h-[1.375rem]">
                <div className="w-10 flex-shrink-0 border-r border-border" />
                <div className="w-1 flex-shrink-0" />
                <div className="flex-1" />
              </div>
            )
          )}
        </div>

        {/* Modified side */}
        <div className="flex-1">
          <div className="sticky top-0 px-3 py-1.5 text-xs text-muted-foreground bg-muted/80 border-b border-border backdrop-blur">
            Working Tree - {file.filepath}
          </div>
          {currentLines.map((line, idx) => {
            const lineNum = idx + 1;
            const hunk = changedCurrent.get(lineNum);
            const isChanged = !!hunk;
            const lineComments = commentsByLine.get(lineNum) || [];
            const hasComments = lineComments.length > 0;
            const hasApproved = lineComments.some((c) => c.status === "approved");
            const hasChangeRequest = lineComments.some(
              (c) => c.status === "change-requested"
            );

            return (
              <div key={lineNum} className="relative">
                <div
                  ref={(el) => {
                    if (el) lineRefs.current.set(lineNum, el);
                  }}
                  className={cn(
                    "flex group",
                    isChanged && "bg-diff-add-bg/30"
                  )}
                >
                  <div
                    className={cn(
                      "w-10 flex-shrink-0 text-right pr-2 select-none border-r border-border",
                      isChanged
                        ? "text-muted-foreground"
                        : "text-muted-foreground/60"
                    )}
                  >
                    {lineNum}
                  </div>

                  {/* Comment icon area */}
                  <div className="w-6 flex-shrink-0 flex items-center justify-center">
                    {hasComments ? (
                      <button
                        type="button"
                        onClick={() => onLineClick(lineNum)}
                        className={cn(
                          "p-0.5 rounded",
                          hasChangeRequest
                            ? "text-status-deleted"
                            : hasApproved
                              ? "text-status-added"
                              : "text-primary"
                        )}
                      >
                        <MessageSquare className="h-3.5 w-3.5 fill-current" />
                      </button>
                    ) : (
                      <button
                        type="button"
                        onClick={() => onLineClick(lineNum)}
                        className="p-0.5 rounded text-muted-foreground/30 opacity-0 group-hover:opacity-100 hover:text-muted-foreground transition-opacity"
                      >
                        <MessageSquare className="h-3.5 w-3.5" />
                      </button>
                    )}
                  </div>

                  <div
                    className={cn(
                      "w-1 flex-shrink-0",
                      isChanged && "bg-status-added"
                    )}
                  />
                  <div
                    className={cn(
                      "flex-1 px-3 whitespace-pre",
                      isChanged && "text-diff-add"
                    )}
                    contentEditable
                    suppressContentEditableWarning
                    spellCheck={false}
                  >
                    {line || " "}
                  </div>
                </div>

                {/* Comment popover */}
                {activeCommentLine === lineNum && (
                  <CommentPopover
                    lineNumber={lineNum}
                    existingComments={comments}
                    isOpen={true}
                    onClose={onCancelComment}
                    onSubmit={(text, status) =>
                      onAddComment(lineNum, text, status, hunk?.id)
                    }
                    anchorRef={{ current: lineRefs.current.get(lineNum) || null }}
                    hunkId={hunk?.id}
                  />
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

// Empty state
function EmptyState({
  files,
  onOpenFile,
}: {
  files: FileData[];
  onOpenFile: (file: FileData) => void;
}) {
  return (
    <div className="h-full flex items-center justify-center text-muted-foreground">
      <div className="text-center space-y-3">
        <p className="text-sm">No file open</p>
        <div className="space-y-1">
          <p className="text-xs">Changed files:</p>
          {files.map((f) => (
            <button
              key={f.id}
              type="button"
              onClick={() => onOpenFile(f)}
              className="block w-full text-left px-3 py-1 text-xs text-primary hover:bg-muted rounded font-mono"
            >
              {f.filepath}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
