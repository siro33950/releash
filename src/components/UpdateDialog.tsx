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
import type { UpdateCheckResult } from "@/hooks/useUpdateChecker";

interface UpdateDialogProps {
	update: UpdateCheckResult;
}

export function UpdateDialog({ update }: UpdateDialogProps) {
	const { status, updateInfo, progress, error, downloadAndInstall, dismiss } =
		update;

	if (status === "idle" || status === "checking") {
		return null;
	}

	if (status === "error") {
		return (
			<AlertDialog open>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Update Error</AlertDialogTitle>
						<AlertDialogDescription>
							{error ?? "An unknown error occurred."}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel onClick={dismiss}>Close</AlertDialogCancel>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		);
	}

	if (status === "downloading") {
		return (
			<AlertDialog open>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Downloading Update...</AlertDialogTitle>
						<AlertDialogDescription>
							Please wait while the update is being downloaded.
						</AlertDialogDescription>
					</AlertDialogHeader>
					<div className="w-full bg-muted rounded-full h-2 overflow-hidden">
						<div
							className="bg-primary h-full transition-all duration-300"
							style={{ width: `${progress}%` }}
							role="progressbar"
							aria-valuenow={progress}
							aria-valuemin={0}
							aria-valuemax={100}
						/>
					</div>
					<p className="text-xs text-muted-foreground text-center">
						{progress}%
					</p>
				</AlertDialogContent>
			</AlertDialog>
		);
	}

	return (
		<AlertDialog open>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Update Available</AlertDialogTitle>
					<AlertDialogDescription>
						Version {updateInfo?.version} is available.
						{updateInfo?.notes && (
							<span className="block mt-2 whitespace-pre-wrap">
								{updateInfo.notes}
							</span>
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={dismiss}>Later</AlertDialogCancel>
					<AlertDialogAction onClick={downloadAndInstall}>
						Update Now
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
