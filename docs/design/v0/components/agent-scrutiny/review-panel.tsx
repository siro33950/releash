"use client";

import { useState } from "react";
import { cn } from "@/lib/utils";
import {
  MessageSquare,
  Send,
  ThumbsUp,
  ThumbsDown,
  RotateCcw,
  CheckCircle2,
  AlertCircle,
} from "lucide-react";
import { Button } from "@/components/ui/button";

interface ReviewComment {
  id: string;
  filename: string;
  line: number;
  content: string;
  type: "suggestion" | "question" | "approval";
  resolved: boolean;
}

const mockComments: ReviewComment[] = [
  {
    id: "1",
    filename: "src/components/Button.tsx",
    line: 15,
    content:
      "Consider adding a 'link' variant that renders as an anchor tag while maintaining button styling.",
    type: "suggestion",
    resolved: false,
  },
  {
    id: "2",
    filename: "src/hooks/useAuth.ts",
    line: 11,
    content:
      "Good improvement! Type safety will help catch errors at compile time.",
    type: "approval",
    resolved: true,
  },
  {
    id: "3",
    filename: "src/components/Input.tsx",
    line: 8,
    content:
      "Should we add support for error states with a red border and error message?",
    type: "question",
    resolved: false,
  },
];

export function ReviewPanel() {
  const [comments, setComments] = useState<ReviewComment[]>(mockComments);
  const [newComment, setNewComment] = useState("");
  const [instruction, setInstruction] = useState("");

  const unresolvedCount = comments.filter((c) => !c.resolved).length;

  const toggleResolved = (id: string) => {
    setComments((prev) =>
      prev.map((c) => (c.id === id ? { ...c, resolved: !c.resolved } : c))
    );
  };

  const getTypeIcon = (type: ReviewComment["type"]) => {
    switch (type) {
      case "suggestion":
        return <AlertCircle className="h-4 w-4 text-status-modified" />;
      case "question":
        return <MessageSquare className="h-4 w-4 text-primary" />;
      case "approval":
        return <CheckCircle2 className="h-4 w-4 text-status-added" />;
    }
  };

  return (
    <div className="h-full flex flex-col bg-sidebar">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-sidebar-border">
        <div className="flex items-center gap-2">
          <MessageSquare className="h-4 w-4 text-primary" />
          <span className="text-sm font-medium">Review</span>
          {unresolvedCount > 0 && (
            <span className="px-1.5 py-0.5 text-[10px] font-medium bg-primary text-primary-foreground rounded-full">
              {unresolvedCount}
            </span>
          )}
        </div>
      </div>

      {/* Review summary */}
      <div className="px-3 py-3 border-b border-sidebar-border bg-card/50">
        <div className="flex items-center justify-between mb-2">
          <span className="text-xs text-muted-foreground">Review Status</span>
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="sm" className="h-6 text-xs text-status-added">
              <ThumbsUp className="h-3 w-3 mr-1" />
              Approve
            </Button>
            <Button variant="ghost" size="sm" className="h-6 text-xs text-status-deleted">
              <ThumbsDown className="h-3 w-3 mr-1" />
              Request
            </Button>
          </div>
        </div>
        <div className="flex items-center gap-4 text-xs">
          <span className="text-muted-foreground">
            <span className="text-status-added font-medium">{comments.filter(c => c.resolved).length}</span> resolved
          </span>
          <span className="text-muted-foreground">
            <span className="text-status-modified font-medium">{unresolvedCount}</span> pending
          </span>
        </div>
      </div>

      {/* Comments list */}
      <div className="flex-1 overflow-auto p-3 space-y-3">
        {comments.map((comment) => (
          <div
            key={comment.id}
            className={cn(
              "p-3 rounded-lg border transition-opacity",
              comment.resolved
                ? "border-border/50 opacity-60"
                : "border-border bg-card"
            )}
          >
            <div className="flex items-start gap-2 mb-2">
              {getTypeIcon(comment.type)}
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-mono text-primary truncate">
                    {comment.filename}
                  </span>
                  <span className="text-[10px] text-muted-foreground">
                    Line {comment.line}
                  </span>
                </div>
              </div>
              <Button
                variant="ghost"
                size="sm"
                className="h-5 w-5 p-0"
                onClick={() => toggleResolved(comment.id)}
              >
                {comment.resolved ? (
                  <RotateCcw className="h-3 w-3 text-muted-foreground" />
                ) : (
                  <CheckCircle2 className="h-3 w-3 text-muted-foreground" />
                )}
              </Button>
            </div>
            <p className="text-xs leading-relaxed">{comment.content}</p>
          </div>
        ))}
      </div>

      {/* Agent instruction input */}
      <div className="p-3 border-t border-sidebar-border">
        <div className="mb-2">
          <span className="text-xs font-medium text-muted-foreground">
            Send instruction to agent
          </span>
        </div>
        <div className="flex gap-2">
          <textarea
            value={instruction}
            onChange={(e) => setInstruction(e.target.value)}
            placeholder="Type instruction for the AI agent..."
            className="flex-1 h-16 p-2 text-xs bg-input border border-border rounded resize-none focus:outline-none focus:ring-1 focus:ring-primary"
          />
        </div>
        <Button
          size="sm"
          className="w-full mt-2 h-7 text-xs"
          disabled={!instruction.trim()}
        >
          <Send className="h-3 w-3 mr-1" />
          Send to Terminal
        </Button>
      </div>
    </div>
  );
}
