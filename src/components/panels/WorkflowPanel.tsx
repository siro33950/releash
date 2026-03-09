import { CommentList } from "@/components/panels/CommentList";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { Thread } from "@/types/thread";

interface WorkflowPanelProps {
	timelineContent: React.ReactNode;
	actions?: React.ReactNode;
	threads: Thread[];
	onThreadClick?: (filePath: string, lineNumber: number) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
}

export function WorkflowPanel({
	timelineContent,
	actions,
	threads,
	onThreadClick,
	onDeleteThread,
	onResolveThread,
}: WorkflowPanelProps) {
	return (
		<div className="flex flex-col h-full">
			<Tabs defaultValue="timeline" className="flex flex-col h-full gap-0">
				<div className="flex items-center justify-between px-2 py-1 border-b border-border shrink-0">
					<TabsList variant="line" className="h-[28px] px-0">
						<TabsTrigger value="timeline" className="text-xs px-2 py-0.5">
							Timeline
						</TabsTrigger>
						<TabsTrigger value="comments" className="text-xs px-2 py-0.5">
							Comments
						</TabsTrigger>
					</TabsList>
					{actions && <div className="flex items-center gap-1">{actions}</div>}
				</div>
				<TabsContent value="timeline" className="flex-1 m-0 overflow-hidden">
					{timelineContent}
				</TabsContent>
				<TabsContent value="comments" className="flex-1 m-0 overflow-hidden">
					<CommentList
						threads={threads}
						onThreadClick={onThreadClick}
						onDeleteThread={onDeleteThread}
						onResolveThread={onResolveThread}
					/>
				</TabsContent>
			</Tabs>
		</div>
	);
}
