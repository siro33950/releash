import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Input } from "@/components/ui/input";

interface SavingConflictDialogProps {
	open: boolean;
	onOpenChange: (o: boolean) => void;
	onOverwrite: () => void;
}

export function SavingConflictDialog({
	open,
	onOpenChange,
	onOverwrite,
}: SavingConflictDialogProps) {
	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>External Change Conflict</AlertDialogTitle>
					<AlertDialogDescription>
						This file has been modified externally. Do you want to overwrite it?
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={() => onOpenChange(false)}>
						Cancel
					</AlertDialogCancel>
					<AlertDialogAction onClick={onOverwrite}>Overwrite</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

interface GitErrorDialogProps {
	error: string | null;
	onOpenChange: (o: boolean) => void;
	onDismiss: () => void;
}

export function GitErrorDialog({
	error,
	onOpenChange,
	onDismiss,
}: GitErrorDialogProps) {
	return (
		<AlertDialog open={!!error} onOpenChange={onOpenChange}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Git Error</AlertDialogTitle>
					<AlertDialogDescription>{error}</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogAction onClick={onDismiss}>OK</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

interface DiscardAllDialogProps {
	open: boolean;
	onOpenChange: (o: boolean) => void;
	onDiscard: () => void;
}

export function DiscardAllDialog({
	open,
	onOpenChange,
	onDiscard,
}: DiscardAllDialogProps) {
	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Discard All Changes</AlertDialogTitle>
					<AlertDialogDescription>
						Are you sure you want to discard all uncommitted changes? This
						action cannot be undone.
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={() => onOpenChange(false)}>
						Cancel
					</AlertDialogCancel>
					<AlertDialogAction variant="destructive" onClick={onDiscard}>
						Discard All
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

interface CreateBranchDialogProps {
	open: boolean;
	onOpenChange: (o: boolean) => void;
	branchName: string;
	onBranchNameChange: (name: string) => void;
	onCreate: () => void;
}

export function CreateBranchDialog({
	open,
	onOpenChange,
	branchName,
	onBranchNameChange,
	onCreate,
}: CreateBranchDialogProps) {
	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Create Branch</AlertDialogTitle>
					<AlertDialogDescription>
						Enter a name for the new branch.
					</AlertDialogDescription>
				</AlertDialogHeader>
				<Input
					value={branchName}
					onChange={(e) => onBranchNameChange(e.target.value)}
					placeholder="Branch name"
					autoFocus
					onKeyDown={(e) => {
						if (e.key === "Enter" && branchName.trim()) onCreate();
					}}
				/>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={() => onOpenChange(false)}>
						Cancel
					</AlertDialogCancel>
					<AlertDialogAction onClick={onCreate} disabled={!branchName.trim()}>
						Create
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
