import { describe, expect, it } from "vitest";
import type { FileNode, FileStatus } from "@/types/file-tree";
import { applyStatusToTree } from "./applyStatusToTree";

describe("applyStatusToTree", () => {
	it("should apply file status from statusMap", () => {
		const tree: FileNode[] = [
			{ name: "a.txt", path: "/repo/a.txt", type: "file" },
			{ name: "b.txt", path: "/repo/b.txt", type: "file" },
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/a.txt", "modified"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBe("modified");
		expect(result[1].status).toBeUndefined();
	});

	it("should propagate modified status to parent folder", () => {
		const tree: FileNode[] = [
			{
				name: "src",
				path: "/repo/src",
				type: "folder",
				children: [
					{ name: "main.ts", path: "/repo/src/main.ts", type: "file" },
				],
			},
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/src/main.ts", "modified"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBe("modified");
		expect(result[0].children?.[0].status).toBe("modified");
	});

	it("should not propagate status when no children have changes", () => {
		const tree: FileNode[] = [
			{
				name: "src",
				path: "/repo/src",
				type: "folder",
				children: [
					{ name: "main.ts", path: "/repo/src/main.ts", type: "file" },
				],
			},
		];
		const statusMap = new Map<string, FileStatus>();

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBeNull();
	});

	it("should handle untracked files", () => {
		const tree: FileNode[] = [
			{ name: "new.txt", path: "/repo/new.txt", type: "file" },
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/new.txt", "untracked"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBe("untracked");
	});

	it("should handle nested folder propagation", () => {
		const tree: FileNode[] = [
			{
				name: "src",
				path: "/repo/src",
				type: "folder",
				children: [
					{
						name: "components",
						path: "/repo/src/components",
						type: "folder",
						children: [
							{
								name: "App.tsx",
								path: "/repo/src/components/App.tsx",
								type: "file",
							},
						],
					},
				],
			},
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/src/components/App.tsx", "added"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBe("modified");
		expect(result[0].children?.[0].status).toBe("modified");
		expect(result[0].children?.[0].children?.[0].status).toBe("added");
	});

	it("should return same node reference when status unchanged", () => {
		const node: FileNode = { name: "a.txt", path: "/repo/a.txt", type: "file" };
		const tree: FileNode[] = [node];
		const statusMap = new Map<string, FileStatus>();

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0]).toBe(node);
	});
});
