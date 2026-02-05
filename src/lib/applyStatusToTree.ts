import type { FileNode, FileStatus } from "@/types/file-tree";

export function applyStatusToTree(
	nodes: FileNode[],
	statusMap: Map<string, FileStatus>,
): FileNode[] {
	return nodes.map((node) => {
		if (node.type === "file") {
			const status = statusMap.get(node.path) ?? null;
			if (status === (node.status ?? null)) return node;
			return { ...node, status };
		}

		const children = node.children
			? applyStatusToTree(node.children, statusMap)
			: undefined;

		const hasChangedChild = children?.some((child) => child.status != null);
		const status: FileStatus = hasChangedChild ? "modified" : null;

		if (status === (node.status ?? null) && children === node.children)
			return node;
		return { ...node, status, children };
	});
}
