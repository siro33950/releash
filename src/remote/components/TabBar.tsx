import type { Tab } from "../hooks/useRemoteNavigation";

interface TabDefinition {
	id: Tab;
	label: string;
	icon: React.ComponentType<{ className?: string }>;
}

interface TabBarProps {
	tabs: TabDefinition[];
	activeTab: Tab;
	onTabChange: (tab: Tab) => void;
}

export function TabBar({ tabs, activeTab, onTabChange }: TabBarProps) {
	return (
		<nav className="flex shrink-0 border-t border-border bg-card">
			{tabs.map((tab) => {
				const Icon = tab.icon;
				const isActive = activeTab === tab.id;
				return (
					<button
						key={tab.id}
						type="button"
						className={`flex-1 flex flex-col items-center justify-center h-12 gap-0.5 transition-colors ${
							isActive
								? "text-primary border-t-2 border-primary"
								: "text-muted-foreground"
						}`}
						onClick={() => onTabChange(tab.id)}
					>
						<Icon className="h-4 w-4" />
						<span className="text-[10px]">{tab.label}</span>
					</button>
				);
			})}
		</nav>
	);
}
