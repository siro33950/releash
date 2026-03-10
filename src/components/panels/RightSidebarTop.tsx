import {
	FileDiff,
	FolderTree,
	GitPullRequestArrow,
	ListTree,
	MessageSquare,
	Search,
	Timer,
} from "lucide-react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

import type { RightTopTab } from "@/types/sidebar";

type TabMode = "editor" | "workflow";

interface TabEntry {
	key: RightTopTab;
	icon: React.ElementType;
	label: string;
	mode: TabMode;
}

interface RightSidebarTopProps {
	activeTab: RightTopTab;
	onTabChange: (tab: RightTopTab) => void;
	mode: TabMode;
	explorerContent: React.ReactNode;
	changesContent: React.ReactNode;
	searchContent: React.ReactNode;
	prContent: React.ReactNode;
	symbolsContent?: React.ReactNode;
	planTimelineContent?: React.ReactNode;
	planCommentContent?: React.ReactNode;
}

const tabs: TabEntry[] = [
	{ key: "changes", icon: FileDiff, label: "Changes", mode: "editor" },
	{ key: "explorer", icon: FolderTree, label: "Explorer", mode: "editor" },
	{ key: "search", icon: Search, label: "Search", mode: "editor" },
	{ key: "symbols", icon: ListTree, label: "Symbols", mode: "editor" },
	{
		key: "pr",
		icon: GitPullRequestArrow,
		label: "Pull Requests",
		mode: "editor",
	},
	{
		key: "plan-timeline",
		icon: Timer,
		label: "Plan Timeline",
		mode: "workflow",
	},
	{
		key: "plan-comment",
		icon: MessageSquare,
		label: "Plan Comments",
		mode: "workflow",
	},
];

export function RightSidebarTop({
	activeTab,
	onTabChange,
	mode,
	explorerContent,
	changesContent,
	searchContent,
	prContent,
	symbolsContent,
	planTimelineContent,
	planCommentContent,
}: RightSidebarTopProps) {
	const contentMap: Record<RightTopTab, React.ReactNode> = {
		explorer: explorerContent,
		changes: changesContent,
		search: searchContent,
		pr: prContent,
		symbols: symbolsContent ?? null,
		"plan-timeline": planTimelineContent ?? null,
		"plan-comment": planCommentContent ?? null,
	};

	const visibleTabs = tabs.filter((t) => t.mode === mode);

	return (
		<div className="flex flex-col h-full">
			<Tabs
				value={activeTab}
				onValueChange={(val) => onTabChange(val as RightTopTab)}
				className="flex flex-col h-full"
			>
				<TabsList variant="line" aria-label="Right sidebar tabs">
					{visibleTabs.map(({ key, icon: Icon, label }) => (
						<TabsTrigger
							key={key}
							value={key}
							aria-label={label}
							className="px-2.5"
						>
							<span className="inline-flex items-center">
								<Icon className="size-3.5" />
							</span>
						</TabsTrigger>
					))}
				</TabsList>
				{visibleTabs.map(({ key }) => (
					<TabsContent key={key} value={key} className="flex-1 overflow-hidden">
						{contentMap[key]}
					</TabsContent>
				))}
			</Tabs>
		</div>
	);
}
