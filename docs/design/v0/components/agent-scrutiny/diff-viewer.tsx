"use client";

import { useState } from "react";
import { cn } from "@/lib/utils";
import { ChevronDown, ChevronRight, GitBranch, Check, X } from "lucide-react";
import { Button } from "@/components/ui/button";

interface DiffLine {
  type: "context" | "add" | "remove" | "header";
  content: string;
  oldLineNumber?: number;
  newLineNumber?: number;
}

interface DiffFile {
  filename: string;
  status: "modified" | "added" | "deleted";
  additions: number;
  deletions: number;
  lines: DiffLine[];
}

const mockDiffs: DiffFile[] = [
  {
    filename: "src/components/Button.tsx",
    status: "modified",
    additions: 12,
    deletions: 5,
    lines: [
      { type: "header", content: "@@ -1,15 +1,22 @@" },
      {
        type: "context",
        content: 'import { cn } from "@/lib/utils";',
        oldLineNumber: 1,
        newLineNumber: 1,
      },
      {
        type: "remove",
        content: 'import { ButtonHTMLAttributes } from "react";',
        oldLineNumber: 2,
      },
      {
        type: "add",
        content:
          'import { ButtonHTMLAttributes, forwardRef, type ForwardedRef } from "react";',
        newLineNumber: 2,
      },
      {
        type: "add",
        content: 'import { cva, type VariantProps } from "class-variance-authority";',
        newLineNumber: 3,
      },
      { type: "context", content: "", oldLineNumber: 3, newLineNumber: 4 },
      {
        type: "remove",
        content: "interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {",
        oldLineNumber: 4,
      },
      {
        type: "remove",
        content: "  variant?: 'primary' | 'secondary';",
        oldLineNumber: 5,
      },
      {
        type: "remove",
        content: "  size?: 'sm' | 'md' | 'lg';",
        oldLineNumber: 6,
      },
      { type: "add", content: "const buttonVariants = cva(", newLineNumber: 5 },
      {
        type: "add",
        content: '  "inline-flex items-center justify-center rounded-md font-medium",',
        newLineNumber: 6,
      },
      { type: "add", content: "  {", newLineNumber: 7 },
      { type: "add", content: "    variants: {", newLineNumber: 8 },
      {
        type: "add",
        content: "      variant: {",
        newLineNumber: 9,
      },
      {
        type: "add",
        content: '        default: "bg-primary text-primary-foreground hover:bg-primary/90",',
        newLineNumber: 10,
      },
      {
        type: "add",
        content: '        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/80",',
        newLineNumber: 11,
      },
      { type: "context", content: "}", oldLineNumber: 7, newLineNumber: 12 },
    ],
  },
  {
    filename: "src/components/Input.tsx",
    status: "added",
    additions: 25,
    deletions: 0,
    lines: [
      { type: "header", content: "@@ -0,0 +1,25 @@" },
      {
        type: "add",
        content: '"use client";',
        newLineNumber: 1,
      },
      { type: "add", content: "", newLineNumber: 2 },
      {
        type: "add",
        content: 'import { forwardRef, type InputHTMLAttributes } from "react";',
        newLineNumber: 3,
      },
      {
        type: "add",
        content: 'import { cn } from "@/lib/utils";',
        newLineNumber: 4,
      },
      { type: "add", content: "", newLineNumber: 5 },
      {
        type: "add",
        content: "export interface InputProps extends InputHTMLAttributes<HTMLInputElement> {}",
        newLineNumber: 6,
      },
    ],
  },
  {
    filename: "src/hooks/useAuth.ts",
    status: "modified",
    additions: 8,
    deletions: 3,
    lines: [
      { type: "header", content: "@@ -10,8 +10,13 @@" },
      {
        type: "context",
        content: "export function useAuth() {",
        oldLineNumber: 10,
        newLineNumber: 10,
      },
      {
        type: "remove",
        content: "  const [user, setUser] = useState(null);",
        oldLineNumber: 11,
      },
      {
        type: "add",
        content: "  const [user, setUser] = useState<User | null>(null);",
        newLineNumber: 11,
      },
      {
        type: "add",
        content: "  const [loading, setLoading] = useState(true);",
        newLineNumber: 12,
      },
      {
        type: "add",
        content: "  const [error, setError] = useState<Error | null>(null);",
        newLineNumber: 13,
      },
      {
        type: "context",
        content: "",
        oldLineNumber: 12,
        newLineNumber: 14,
      },
    ],
  },
];

interface DiffFileViewProps {
  file: DiffFile;
  isExpanded: boolean;
  onToggle: () => void;
}

function DiffFileView({ file, isExpanded, onToggle }: DiffFileViewProps) {
  return (
    <div className="border border-border rounded-md overflow-hidden mb-3">
      <button
        type="button"
        onClick={onToggle}
        className="flex items-center w-full px-3 py-2 bg-card hover:bg-muted transition-colors text-left"
      >
        {isExpanded ? (
          <ChevronDown className="h-4 w-4 mr-2 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 mr-2 text-muted-foreground" />
        )}
        <span className="font-mono text-sm flex-1 truncate">{file.filename}</span>
        <span className="flex items-center gap-2 text-xs">
          <span className="text-diff-add">+{file.additions}</span>
          <span className="text-diff-remove">-{file.deletions}</span>
        </span>
      </button>
      {isExpanded && (
        <div className="bg-terminal-bg overflow-x-auto">
          <table className="w-full text-sm font-mono">
            <tbody>
              {file.lines.map((line, index) => (
                <tr
                  key={index}
                  className={cn(
                    line.type === "add" && "bg-diff-add-bg",
                    line.type === "remove" && "bg-diff-remove-bg"
                  )}
                >
                  <td className="w-12 text-right pr-2 text-muted-foreground select-none border-r border-border">
                    {line.oldLineNumber ?? ""}
                  </td>
                  <td className="w-12 text-right pr-2 text-muted-foreground select-none border-r border-border">
                    {line.newLineNumber ?? ""}
                  </td>
                  <td className="w-6 text-center select-none">
                    {line.type === "add" && (
                      <span className="text-diff-add">+</span>
                    )}
                    {line.type === "remove" && (
                      <span className="text-diff-remove">-</span>
                    )}
                  </td>
                  <td className="pr-4">
                    <pre
                      className={cn(
                        "whitespace-pre",
                        line.type === "header" && "text-primary font-semibold",
                        line.type === "add" && "text-diff-add",
                        line.type === "remove" && "text-diff-remove"
                      )}
                    >
                      {line.content}
                    </pre>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

type DiffBase = "HEAD" | "HEAD~1" | "HEAD~5" | "staged" | "working";

const baseOptions: { value: DiffBase; label: string }[] = [
  { value: "HEAD", label: "HEAD" },
  { value: "HEAD~1", label: "HEAD~1" },
  { value: "HEAD~5", label: "HEAD~5" },
  { value: "staged", label: "Staged" },
  { value: "working", label: "Working Tree" },
];

interface DiffViewerProps {
  base?: DiffBase;
  onBaseChange?: (base: DiffBase) => void;
}

export function DiffViewer({ base: controlledBase, onBaseChange }: DiffViewerProps) {
  const [internalBase, setInternalBase] = useState<DiffBase>("HEAD");
  const base = controlledBase ?? internalBase;
  
  const handleBaseChange = (newBase: DiffBase) => {
    if (onBaseChange) {
      onBaseChange(newBase);
    } else {
      setInternalBase(newBase);
    }
  };

  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(
    new Set(mockDiffs.map((d) => d.filename))
  );

  const toggleFile = (filename: string) => {
    const next = new Set(expandedFiles);
    if (next.has(filename)) {
      next.delete(filename);
    } else {
      next.add(filename);
    }
    setExpandedFiles(next);
  };

  const totalAdditions = mockDiffs.reduce((sum, d) => sum + d.additions, 0);
  const totalDeletions = mockDiffs.reduce((sum, d) => sum + d.deletions, 0);

  return (
    <div className="h-full flex flex-col bg-background">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-border bg-card">
        <div className="flex items-center gap-3">
          <GitBranch className="h-4 w-4 text-primary" />
          <span className="text-xs text-muted-foreground">Base:</span>
          <select
            value={base}
            onChange={(e) => handleBaseChange(e.target.value as DiffBase)}
            className="bg-muted border border-border rounded px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
          >
            {baseOptions.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
          <span className="text-muted-foreground">...</span>
          <span className="font-mono text-xs text-primary">Working Tree</span>
          <span className="text-xs text-muted-foreground ml-2">
            {mockDiffs.length} files
          </span>
          <span className="text-xs text-diff-add">+{totalAdditions}</span>
          <span className="text-xs text-diff-remove">-{totalDeletions}</span>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs text-diff-add hover:text-diff-add hover:bg-diff-add-bg"
          >
            <Check className="h-3 w-3 mr-1" />
            Accept All
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs text-diff-remove hover:text-diff-remove hover:bg-diff-remove-bg"
          >
            <X className="h-3 w-3 mr-1" />
            Reject All
          </Button>
        </div>
      </div>

      {/* Diff content */}
      <div className="flex-1 overflow-auto p-4">
        {mockDiffs.map((file) => (
          <DiffFileView
            key={file.filename}
            file={file}
            isExpanded={expandedFiles.has(file.filename)}
            onToggle={() => toggleFile(file.filename)}
          />
        ))}
      </div>
    </div>
  );
}
