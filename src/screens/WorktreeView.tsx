import { Layout } from "flexlayout-react";
import { Loader2 } from "lucide-react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { ActivityBar } from "@/components/layout/ActivityBar";
import { StatusBar } from "@/components/layout/StatusBar";
import { ViewToolbar } from "@/components/layout/ViewToolbar";
import { ReviewPanel } from "@/components/panels/ReviewPanel";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { TerminalTabPanel } from "@/components/panels/TerminalTabPanel";
import { UnsavedChangesDialog } from "@/components/panels/UnsavedChangesDialog";
import { EditorContext } from "@/contexts/EditorContext";
import { useWorktreeState } from "@/screens/useWorktreeState";
import {
	CreateBranchDialog,
	DiscardAllDialog,
	GitErrorDialog,
	SavingConflictDialog,
} from "@/screens/WorktreeViewDialogs";
import { type AppSettings, buildTerminalCommand } from "@/types/settings";

interface WorktreeViewProps {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	isActive: boolean;
}

export function WorktreeView({
	rootPath,
	settings,
	onSettingsSave,
	isActive,
}: WorktreeViewProps) {
	const s = useWorktreeState({ rootPath, settings, onSettingsSave, isActive });

	return (
		<div className="flex flex-col h-full w-full overflow-hidden bg-background text-foreground">
			<ViewToolbar panels={s.togglePanels} />
			<div className="flex flex-1 overflow-hidden">
				<ActivityBar
					activeItem={s.activeView}
					onItemClick={(id) => {
						if (id === "settings") {
							s.dispatchUI({ type: "SET_SETTINGS_OPEN", open: true });
						} else {
							s.dispatchEditor({ type: "SET_ACTIVE_VIEW", view: id });
						}
					}}
				/>
				{!s.ready ? (
					<div className="flex-1 flex items-center justify-center">
						<Loader2 className="size-6 text-muted-foreground animate-spin" />
					</div>
				) : (
					<EditorContext.Provider value={s.editorContextValue}>
						<Group orientation="horizontal" className="flex-1">
							<Panel
								panelRef={s.sidebarPanelRef}
								id="sidebar"
								defaultSize="20%"
								minSize="10%"
								collapsible
								collapsedSize="0%"
								onResize={s.handleSidebarResize}
							>
								<div className="h-full overflow-hidden border-r border-border">
									{s.sidebarContent}
								</div>
							</Panel>
							<Separator />
							<Panel id="center" minSize="20%">
								<Group orientation="vertical">
									<Panel id="editor" minSize="20%">
										<div
											ref={s.editorDropZoneRef}
											role="application"
											className="h-full relative overflow-hidden"
											onDragOver={s.handleEditorDragOver}
											onDragLeave={s.handleEditorDragLeave}
											onDrop={s.handleEditorDrop}
										>
											<Layout
												model={s.editorLayout.model}
												factory={s.factory}
												onAction={s.editorLayout.onAction}
												onRenderTab={s.onRenderTab}
												onModelChange={s.forceRender}
											/>
											{s.editorDragOver && (
												<div className="absolute inset-0 flex items-center justify-center bg-primary/10 border-2 border-dashed border-primary rounded pointer-events-none">
													<span className="text-sm font-medium text-primary bg-background/80 px-3 py-1.5 rounded">
														ドロップしてファイルを開く
													</span>
												</div>
											)}
										</div>
									</Panel>
									<Separator />
									<Panel
										panelRef={s.reviewPanelRef}
										id="review"
										defaultSize="30%"
										minSize="10%"
										collapsible
										collapsedSize="0%"
										onResize={s.handleReviewResize}
									>
										<div className="h-full overflow-hidden border-t border-border">
											<ReviewPanel
												comments={s.comments}
												onCommentClick={s.handleCommentClick}
												onDeleteComment={s.removeComment}
												onUpdateComment={s.updateComment}
												onSendToTerminal={s.handleSendToTerminal}
												onSendComment={s.handleSendComment}
												onCopyComment={s.handleCopyComment}
												showSentComments={s.showSentComments}
												onToggleShowSent={s.toggleShowSentComments}
												cwd={rootPath}
												theme={settings.theme}
											/>
										</div>
									</Panel>
								</Group>
							</Panel>
							<Separator />
							<Panel
								panelRef={s.terminalPanelRef}
								id="terminal"
								defaultSize="30%"
								minSize="10%"
								collapsible
								collapsedSize="0%"
								onResize={s.handleTerminalResize}
							>
								<div className="h-full overflow-hidden border-l border-border">
									<TerminalTabPanel
										ref={s.terminalRef}
										key={rootPath}
										cwd={rootPath}
										theme={settings.theme}
										terminalStartupCommand={buildTerminalCommand(settings)}
										agentType={settings.agent}
									/>
								</div>
							</Panel>
						</Group>
					</EditorContext.Provider>
				)}
			</div>
			<StatusBar
				className="shrink-0"
				branch={s.branch ?? undefined}
				language={s.activeTab?.language}
				encoding={s.activeTab ? "UTF-8" : undefined}
				eol={s.activeTab?.eol}
				agentState={s.agentState}
			/>
			<UnsavedChangesDialog
				open={!!s.closingTabPath}
				fileName={s.closingTab?.name ?? ""}
				onSave={s.handleUnsavedSave}
				onDiscard={s.handleUnsavedDiscard}
				onCancel={s.handleUnsavedCancel}
			/>
			<SavingConflictDialog
				open={!!s.savingConflictPath}
				onOpenChange={(o) => {
					if (!o) s.dispatchUI({ type: "SET_SAVING_CONFLICT", path: null });
				}}
				onOverwrite={() => {
					if (s.savingConflictPath) {
						s.clearExternalChange(s.savingConflictPath);
						s.saveFile(s.savingConflictPath);
					}
					s.dispatchUI({ type: "SET_SAVING_CONFLICT", path: null });
				}}
			/>
			<GitErrorDialog
				error={s.gitError}
				onOpenChange={(o) => {
					if (!o) s.dispatchGit({ type: "SET_GIT_ERROR", error: null });
				}}
				onDismiss={() => s.dispatchGit({ type: "SET_GIT_ERROR", error: null })}
			/>
			<DiscardAllDialog
				open={s.showDiscardConfirm}
				onOpenChange={(o) => {
					if (!o) s.dispatchUI({ type: "SET_DISCARD_CONFIRM", show: false });
				}}
				onDiscard={s.gitActions.executeDiscardAll}
			/>
			<CreateBranchDialog
				open={s.showCreateBranch}
				onOpenChange={(o) => {
					if (!o) s.dispatchUI({ type: "CLOSE_CREATE_BRANCH" });
				}}
				branchName={s.newBranchName}
				onBranchNameChange={(name) =>
					s.dispatchUI({ type: "SET_NEW_BRANCH_NAME", name })
				}
				onCreate={s.gitActions.executeCreateBranch}
			/>
			<SettingsModal
				open={s.isSettingsOpen}
				onOpenChange={(open) =>
					s.dispatchUI({ type: "SET_SETTINGS_OPEN", open })
				}
				settings={settings}
				onSave={onSettingsSave}
			/>
		</div>
	);
}
