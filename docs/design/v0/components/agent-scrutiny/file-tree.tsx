"use client";

import { useState } from "react";
import {
  ChevronRight,
  ChevronDown,
  File,
  Folder,
  FolderOpen,
} from "lucide-react";
import { cn } from "@/lib/utils";

type FileStatus = "modified" | "added" | "deleted" | "untracked" | null;

interface FileNode {
  name: string;
  type: "file" | "folder";
  status?: FileStatus;
  children?: FileNode[];
}

const mockFileTree: FileNode[] = [
  {
    name: "src",
    type: "folder",
    children: [
      {
        name: "components",
        type: "folder",
        children: [
          { name: "Button.tsx", type: "file", status: "modified" },
          { name: "Card.tsx", type: "file" },
          { name: "Input.tsx", type: "file", status: "added" },
        ],
      },
      {
        name: "hooks",
        type: "folder",
        children: [
          { name: "useAuth.ts", type: "file", status: "modified" },
          { name: "useDebounce.ts", type: "file" },
        ],
      },
      { name: "App.tsx", type: "file", status: "modified" },
      { name: "index.tsx", type: "file" },
    ],
  },
  {
    name: "package.json",
    type: "file",
    status: "modified",
  },
  {
    name: "tsconfig.json",
    type: "file",
  },
  {
    name: "README.md",
    type: "file",
    status: "untracked",
  },
];

function getStatusColor(status: FileStatus) {
  switch (status) {
    case "modified":
      return "text-status-modified";
    case "added":
      return "text-status-added";
    case "deleted":
      return "text-status-deleted";
    case "untracked":
      return "text-status-untracked";
    default:
      return "text-sidebar-foreground";
  }
}

function getStatusIndicator(status: FileStatus) {
  switch (status) {
    case "modified":
      return "M";
    case "added":
      return "A";
    case "deleted":
      return "D";
    case "untracked":
      return "U";
    default:
      return null;
  }
}

interface FileTreeItemProps {
  node: FileNode;
  depth?: number;
  selectedFile: string | null;
  onSelectFile: (name: string) => void;
}

function FileTreeItem({
  node,
  depth = 0,
  selectedFile,
  onSelectFile,
}: FileTreeItemProps) {
  const [isExpanded, setIsExpanded] = useState(depth < 2);

  const handleClick = () => {
    if (node.type === "folder") {
      setIsExpanded(!isExpanded);
    } else {
      onSelectFile(node.name);
    }
  };

  const isSelected = selectedFile === node.name;

  return (
    <div>
      <button
        type="button"
        onClick={handleClick}
        className={cn(
          "flex w-full items-center gap-1 px-2 py-1 text-sm hover:bg-sidebar-accent transition-colors",
          isSelected && "bg-sidebar-accent",
          getStatusColor(node.status ?? null)
        )}
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
      >
        {node.type === "folder" ? (
          <>
            {isExpanded ? (
              <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
            )}
            {isExpanded ? (
              <FolderOpen className="h-4 w-4 shrink-0 text-status-modified" />
            ) : (
              <Folder className="h-4 w-4 shrink-0 text-status-modified" />
            )}
          </>
        ) : (
          <>
            <span className="w-4" />
            <File className="h-4 w-4 shrink-0 text-muted-foreground" />
          </>
        )}
        <span className="truncate flex-1 text-left">{node.name}</span>
        {node.status && (
          <span
            className={cn(
              "text-xs font-mono shrink-0",
              getStatusColor(node.status)
            )}
          >
            {getStatusIndicator(node.status)}
          </span>
        )}
      </button>
      {node.type === "folder" && isExpanded && node.children && (
        <div>
          {node.children.map((child) => (
            <FileTreeItem
              key={child.name}
              node={child}
              depth={depth + 1}
              selectedFile={selectedFile}
              onSelectFile={onSelectFile}
            />
          ))}
        </div>
      )}
    </div>
  );
}

interface FileTreeProps {
  selectedFile: string | null;
  onSelectFile: (name: string) => void;
}

export function FileTree({ selectedFile, onSelectFile }: FileTreeProps) {
  return (
    <div className="h-full bg-sidebar text-sidebar-foreground overflow-auto">
      <div className="p-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground border-b border-sidebar-border">
        Explorer
      </div>
      <div className="py-1">
        {mockFileTree.map((node) => (
          <FileTreeItem
            key={node.name}
            node={node}
            selectedFile={selectedFile}
            onSelectFile={onSelectFile}
          />
        ))}
      </div>
    </div>
  );
}
