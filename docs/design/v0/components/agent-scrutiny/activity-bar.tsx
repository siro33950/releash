"use client";

import {
  Files,
  GitBranch,
  Search,
  MessageSquare,
  Settings,
  TerminalIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

export type ActivityView =
  | "explorer"
  | "git"
  | "search"
  | "review"
  | "terminal"
  | "settings";

interface ActivityBarProps {
  activeView: ActivityView;
  onViewChange: (view: ActivityView) => void;
}

const activities = [
  { id: "explorer" as const, icon: Files, label: "Explorer" },
  { id: "git" as const, icon: GitBranch, label: "Source Control" },
  { id: "search" as const, icon: Search, label: "Search" },
  { id: "review" as const, icon: MessageSquare, label: "Review", badge: 2 },
];

const bottomActivities = [
  { id: "settings" as const, icon: Settings, label: "Settings" },
];

export function ActivityBar({ activeView, onViewChange }: ActivityBarProps) {
  return (
    <div className="w-12 h-full flex flex-col items-center py-2 bg-sidebar border-r border-sidebar-border">
      <div className="flex-1 flex flex-col items-center gap-1">
        {activities.map((activity) => (
          <Button
            key={activity.id}
            variant="ghost"
            size="sm"
            className={cn(
              "w-10 h-10 p-0 relative",
              activeView === activity.id
                ? "text-foreground bg-sidebar-accent before:absolute before:left-0 before:top-1/2 before:-translate-y-1/2 before:w-0.5 before:h-6 before:bg-primary before:rounded-r"
                : "text-muted-foreground hover:text-foreground"
            )}
            onClick={() => onViewChange(activity.id)}
            title={activity.label}
          >
            <activity.icon className="h-5 w-5" />
            {activity.badge && (
              <span className="absolute top-1 right-1 w-4 h-4 flex items-center justify-center text-[10px] font-medium bg-primary text-primary-foreground rounded-full">
                {activity.badge}
              </span>
            )}
          </Button>
        ))}
      </div>

      <div className="flex flex-col items-center gap-1">
        {bottomActivities.map((activity) => (
          <Button
            key={activity.id}
            variant="ghost"
            size="sm"
            className={cn(
              "w-10 h-10 p-0",
              activeView === activity.id
                ? "text-foreground bg-sidebar-accent"
                : "text-muted-foreground hover:text-foreground"
            )}
            onClick={() => onViewChange(activity.id)}
            title={activity.label}
          >
            <activity.icon className="h-5 w-5" />
          </Button>
        ))}
      </div>
    </div>
  );
}
