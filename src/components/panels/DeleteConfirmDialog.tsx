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

interface DeleteConfirmDialogProps {
	open: boolean;
	itemName: string;
	onConfirm: () => void;
	onCancel: () => void;
}

export function DeleteConfirmDialog({
	open,
	itemName,
	onConfirm,
	onCancel,
}: DeleteConfirmDialogProps) {
	return (
		<AlertDialog open={open} onOpenChange={(o) => !o && onCancel()}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>削除の確認</AlertDialogTitle>
					<AlertDialogDescription>
						「{itemName}」を削除しますか？この操作は取り消せません。
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={onCancel}>キャンセル</AlertDialogCancel>
					<AlertDialogAction onClick={onConfirm}>削除</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
