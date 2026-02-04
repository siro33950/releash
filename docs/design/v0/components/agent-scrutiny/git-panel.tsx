"use client";

import { useState } from "react";
import { cn } from "@/lib/utils";
import {
  GitBranch,
  GitCommit,
  RefreshCw,
  ChevronDown,
  Check,
  Plus,
  Minus,
} from "lucide-react";
import { Button } from "@/components/ui/button";

interface GitStatus {
  branch: string;
  ahead: number;
  behind: number;
  staged: string[];
  unstaged: string[];
  untracked: string[];
}

interface CommitLog {
  hash: string;
  message: string;
  author: string;
  date: string;
}

const mockStatus: GitStatus = {
  branch: "feature/ai-improvements",
  ahead: 2,
  behind: 0,
  staged: [],
  unstaged: ["src/components/Button.tsx", "src/hooks/useAuth.ts", "src/App.tsx"],
  untracked: ["src/components/Input.tsx"],
};

const mockLog: CommitLog[] = [
  {
    hash: "a1b2c3d",
    message: "Add type safety to Button component",
    author: "AI Agent",
    date: "2 minutes ago",
  },
  {
    hash: "e4f5g6h",
    message: "Refactor useAuth hook",
    author: "AI Agent",
    date: "15 minutes ago",
  },
  {
    hash: "i7j8k9l",
    message: "Initial project setup",
    author: "Developer",
    date: "1 hour ago",
  },
  {
    hash: "m0n1o2p",
    message: "Configure TypeScript",
    author: "Developer",
    date: "2 hours ago",
  },
];

type TabType = "status" | "log";

export function GitPanel() {
  const [activeTab, setActiveTab] = useState<TabType>("status");
  const [commitMessage, setCommitMessage] = useState("");

  return (
    <div className="h-full flex flex-col bg-sidebar">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-sidebar-border">
        <div className="flex items-center gap-2">
          <GitBranch className="h-4 w-4 text-primary" />
          <span className="text-sm font-medium">{mockStatus.branch}</span>
        </div>
        <Button variant="ghost" size="sm" className="h-6 w-6 p-0">
          <RefreshCw className="h-3 w-3" />
        </Button>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-sidebar-border">
        <button
          type="button"
          onClick={() => setActiveTab("status")}
          className={cn(
            "flex-1 px-3 py-2 text-xs font-medium transition-colors",
            activeTab === "status"
              ? "text-foreground border-b-2 border-primary"
              : "text-muted-foreground hover:text-foreground"
          )}
        >
          Status
        </button>
        <button
          type="button"
          onClick={() => setActiveTab("log")}
          className={cn(
            "flex-1 px-3 py-2 text-xs font-medium transition-colors",
            activeTab === "log"
              ? "text-foreground border-b-2 border-primary"
              : "text-muted-foreground hover:text-foreground"
          )}
        >
          History
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-auto">
        {activeTab === "status" ? (
          <div className="p-3 space-y-4">
            {/* Staged changes */}
            <div>
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs font-medium text-muted-foreground uppercase">
                  Staged Changes ({mockStatus.staged.length})
                </span>
              </div>
              {mockStatus.staged.length === 0 && (
                <p className="text-xs text-muted-foreground italic">
                  No staged changes
                </p>
              )}
            </div>

            {/* Unstaged changes */}
            <div>
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs font-medium text-muted-foreground uppercase">
                  Changes ({mockStatus.unstaged.length})
                </span>
                <Button variant="ghost" size="sm" className="h-5 text-xs">
                  <Plus className="h-3 w-3 mr-1" />
                  Stage All
                </Button>
              </div>
              <div className="space-y-1">
                {mockStatus.unstaged.map((file) => (
                  <div
                    key={file}
                    className="flex items-center gap-2 px-2 py-1 text-xs rounded hover:bg-sidebar-accent group"
                  >
                    <span className="text-status-modified font-mono">M</span>
                    <span className="flex-1 truncate font-mono">{file}</span>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-4 w-4 p-0 opacity-0 group-hover:opacity-100"
                    >
                      <Plus className="h-3 w-3" />
                    </Button>
                  </div>
                ))}
              </div>
            </div>

            {/* Untracked files */}
            <div>
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs font-medium text-muted-foreground uppercase">
                  Untracked ({mockStatus.untracked.length})
                </span>
              </div>
              <div className="space-y-1">
                {mockStatus.untracked.map((file) => (
                  <div
                    key={file}
                    className="flex items-center gap-2 px-2 py-1 text-xs rounded hover:bg-sidebar-accent group"
                  >
                    <span className="text-status-added font-mono">A</span>
                    <span className="flex-1 truncate font-mono">{file}</span>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-4 w-4 p-0 opacity-0 group-hover:opacity-100"
                    >
                      <Plus className="h-3 w-3" />
                    </Button>
                  </div>
                ))}
              </div>
            </div>

            {/* Commit input */}
            <div className="pt-2 border-t border-sidebar-border">
              <textarea
                value={commitMessage}
                onChange={(e) => setCommitMessage(e.target.value)}
                placeholder="Commit message..."
                className="w-full h-20 p-2 text-xs bg-input border border-border rounded resize-none focus:outline-none focus:ring-1 focus:ring-primary"
              />
              <Button
                size="sm"
                className="w-full mt-2 h-7 text-xs"
                disabled={!commitMessage.trim()}
              >
                <Check className="h-3 w-3 mr-1" />
                Commit
              </Button>
            </div>
          </div>
        ) : (
          <div className="p-3 space-y-2">
            {mockLog.map((commit) => (
              <div
                key={commit.hash}
                className="flex items-start gap-2 p-2 rounded hover:bg-sidebar-accent cursor-pointer"
              >
                <GitCommit className="h-4 w-4 mt-0.5 text-muted-foreground shrink-0" />
                <div className="flex-1 min-w-0">
                  <p className="text-xs font-medium truncate">{commit.message}</p>
                  <div className="flex items-center gap-2 mt-1">
                    <span className="text-[10px] font-mono text-primary">
                      {commit.hash}
                    </span>
                    <span className="text-[10px] text-muted-foreground">
                      {commit.author}
                    </span>
                    <span className="text-[10px] text-muted-foreground">
                      {commit.date}
                    </span>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
