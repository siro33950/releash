import { Files, GitBranch, GitPullRequest, Search } from "lucide-react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

export type RightTopTab = "explorer" | "changes" | "search" | "pr";

interface RightSidebarTopProps {
	activeTab: RightTopTab;
	onTabChange: (tab: RightTopTab) => void;
	explorerContent: React.ReactNode;
	changesContent: React.ReactNode;
	searchContent: React.ReactNode;
	prContent: React.ReactNode;
}

const tabs: { key: RightTopTab; icon: React.ElementType; label: string }[] = [
	{ key: "changes", icon: GitBranch, label: "Changes" },
	{ key: "explorer", icon: Files, label: "Explorer" },
	{ key: "search", icon: Search, label: "Search" },
	{ key: "pr", icon: GitPullRequest, label: "Pull Requests" },
];

export function RightSidebarTop({
	activeTab,
	onTabChange,
	explorerContent,
	changesContent,
	searchContent,
	prContent,
}: RightSidebarTopProps) {
	const contentMap: Record<RightTopTab, React.ReactNode> = {
		explorer: explorerContent,
		changes: changesContent,
		search: searchContent,
		pr: prContent,
	};

	return (
		<div className="flex flex-col h-full">
			<Tabs
				value={activeTab}
				onValueChange={(val) => onTabChange(val as RightTopTab)}
				className="flex flex-col h-full gap-0"
			>
				<TabsList variant="line" aria-label="Right sidebar tabs">
					{tabs.map(({ key, icon: Icon, label }) => (
						<TabsTrigger key={key} value={key} aria-label={label}>
							<Icon className="size-3.5" />
						</TabsTrigger>
					))}
				</TabsList>
				{tabs.map(({ key }) => (
					<TabsContent key={key} value={key} className="flex-1 overflow-hidden">
						{contentMap[key]}
					</TabsContent>
				))}
			</Tabs>
		</div>
	);
}
