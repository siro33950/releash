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
