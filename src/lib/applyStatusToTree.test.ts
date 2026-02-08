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

	it("should apply ignored status directly to files", () => {
		const tree: FileNode[] = [
			{ name: "ignored.log", path: "/repo/ignored.log", type: "file" },
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/ignored.log", "ignored"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBe("ignored");
	});

	it("should not propagate folder status for ignored-only children", () => {
		const tree: FileNode[] = [
			{
				name: "src",
				path: "/repo/src",
				type: "folder",
				children: [
					{ name: "cache.log", path: "/repo/src/cache.log", type: "file" },
				],
			},
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/src/cache.log", "ignored"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBeNull();
		expect(result[0].children?.[0].status).toBe("ignored");
	});

	it("should propagate status to collapsed folder with changes", () => {
		const tree: FileNode[] = [
			{
				name: "src",
				path: "/repo/src",
				type: "folder",
				children: undefined,
			},
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/src/main.ts", "modified"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBe("modified");
	});

	it("should not propagate status to collapsed folder with only ignored changes", () => {
		const tree: FileNode[] = [
			{
				name: "vendor",
				path: "/repo/vendor",
				type: "folder",
				children: undefined,
			},
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/vendor/lib.js", "ignored"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBeNull();
	});

	it("should propagate ignored from parent folder to children", () => {
		const tree: FileNode[] = [
			{
				name: "node_modules",
				path: "/repo/node_modules",
				type: "folder",
				children: [
					{
						name: "pkg",
						path: "/repo/node_modules/pkg",
						type: "folder",
						children: [
							{
								name: "index.js",
								path: "/repo/node_modules/pkg/index.js",
								type: "file",
							},
						],
					},
				],
			},
		];
		const statusMap = new Map<string, FileStatus>([
			["/repo/node_modules", "ignored"],
		]);

		const result = applyStatusToTree(tree, statusMap);

		expect(result[0].status).toBe("ignored");
		expect(result[0].children?.[0].status).toBe("ignored");
		expect(result[0].children?.[0].children?.[0].status).toBe("ignored");
	});
});
