import { beforeEach, describe, expect, it } from "vitest";
import type { PaneContainer, PaneLeaf, PaneNode } from "@/types/terminal-pane";
import {
	_resetContainerIdCounter,
	closePane,
	countLeaves,
	findNode,
	findParent,
	getAdjacentPane,
	getAllLeaves,
	insertRebalance,
	normalize,
	removalRebalance,
	splitPane,
} from "./paneTree";

function leaf(id: string, label?: string): PaneLeaf {
	return { type: "leaf", id, label: label ?? id, ptyId: null };
}

function container(
	id: string,
	direction: "horizontal" | "vertical",
	children: PaneNode[],
	ratios?: number[],
): PaneContainer {
	return {
		type: "container",
		id,
		direction,
		children,
		ratios: ratios ?? children.map(() => 1 / children.length),
	};
}

describe("paneTree", () => {
	beforeEach(() => {
		_resetContainerIdCounter();
	});

	describe("findNode", () => {
		it("ルートリーフを見つける", () => {
			const tree = leaf("a");
			expect(findNode(tree, "a")).toBe(tree);
		});

		it("ネストされたリーフを見つける", () => {
			const l = leaf("b");
			const tree = container("root", "vertical", [leaf("a"), l]);
			expect(findNode(tree, "b")).toBe(l);
		});

		it("存在しないIDではnullを返す", () => {
			const tree = leaf("a");
			expect(findNode(tree, "nonexistent")).toBeNull();
		});
	});

	describe("findParent", () => {
		it("ルートノードには親がない", () => {
			const tree = leaf("a");
			expect(findParent(tree, "a")).toBeNull();
		});

		it("子の親コンテナとインデックスを返す", () => {
			const tree = container("root", "vertical", [leaf("a"), leaf("b")]);
			const result = findParent(tree, "b");
			expect(result?.parent.id).toBe("root");
			expect(result?.index).toBe(1);
		});
	});

	describe("getAllLeaves", () => {
		it("リーフ単体のツリー", () => {
			const tree = leaf("a");
			expect(getAllLeaves(tree)).toEqual([tree]);
		});

		it("ネストされたツリーの全リーフを取得", () => {
			const tree = container("root", "vertical", [
				leaf("a"),
				container("inner", "horizontal", [leaf("b"), leaf("c")]),
			]);
			const leaves = getAllLeaves(tree);
			expect(leaves.map((l) => l.id)).toEqual(["a", "b", "c"]);
		});
	});

	describe("countLeaves", () => {
		it("リーフ単体は1", () => {
			expect(countLeaves(leaf("a"))).toBe(1);
		});

		it("ネストされたツリーのリーフ数", () => {
			const tree = container("root", "vertical", [
				leaf("a"),
				container("inner", "horizontal", [leaf("b"), leaf("c")]),
			]);
			expect(countLeaves(tree)).toBe(3);
		});
	});

	describe("insertRebalance", () => {
		it("2分割から3分割への再配分", () => {
			const result = insertRebalance([0.5, 0.5]);
			expect(result).toHaveLength(3);
			const sum = result.reduce((a, b) => a + b, 0);
			expect(sum).toBeCloseTo(1.0);
			// 新しい子は1/3
			expect(result[2]).toBeCloseTo(1 / 3);
		});

		it("均等比率からの再配分", () => {
			const result = insertRebalance([1]);
			expect(result).toHaveLength(2);
			expect(result[0]).toBeCloseTo(0.5);
			expect(result[1]).toBeCloseTo(0.5);
		});

		it("不均等比率からの再配分", () => {
			const result = insertRebalance([0.7, 0.3]);
			expect(result).toHaveLength(3);
			const sum = result.reduce((a, b) => a + b, 0);
			expect(sum).toBeCloseTo(1.0);
			// 比率は維持される
			expect(result[0]).toBeGreaterThan(result[1]);
		});
	});

	describe("removalRebalance", () => {
		it("3分割から2分割への再配分", () => {
			const result = removalRebalance([1 / 3, 1 / 3, 1 / 3], 1);
			expect(result).toHaveLength(2);
			const sum = result.reduce((a, b) => a + b, 0);
			expect(sum).toBeCloseTo(1.0);
			expect(result[0]).toBeCloseTo(0.5);
			expect(result[1]).toBeCloseTo(0.5);
		});

		it("2分割から1分割への再配分", () => {
			const result = removalRebalance([0.5, 0.5], 0);
			expect(result).toHaveLength(1);
			expect(result[0]).toBeCloseTo(1.0);
		});

		it("最後の1つを削除すると空配列", () => {
			const result = removalRebalance([1.0], 0);
			expect(result).toHaveLength(0);
		});

		it("不均等比率からの削除でも合計が1.0", () => {
			const result = removalRebalance([0.6, 0.2, 0.2], 0);
			expect(result).toHaveLength(2);
			const sum = result.reduce((a, b) => a + b, 0);
			expect(sum).toBeCloseTo(1.0);
		});
	});

	describe("normalize", () => {
		it("リーフはそのまま返す", () => {
			const tree = leaf("a");
			expect(normalize(tree)).toBe(tree);
		});

		it("単一子のコンテナは子を昇格", () => {
			const child = leaf("a");
			const tree = container("root", "vertical", [child]);
			expect(normalize(tree)).toEqual(child);
		});

		it("同方向コンテナをフラット化", () => {
			const tree = container("root", "vertical", [
				leaf("a"),
				container("inner", "vertical", [leaf("b"), leaf("c")]),
			]);
			const result = normalize(tree) as PaneContainer;
			expect(result.type).toBe("container");
			expect(result.children).toHaveLength(3);
			expect(result.children.map((c) => c.id)).toEqual(["a", "b", "c"]);
		});

		it("異方向コンテナはフラット化しない", () => {
			const tree = container("root", "vertical", [
				leaf("a"),
				container("inner", "horizontal", [leaf("b"), leaf("c")]),
			]);
			const result = normalize(tree) as PaneContainer;
			expect(result.children).toHaveLength(2);
			expect(result.children[1].type).toBe("container");
		});

		it("比率を正規化する", () => {
			const tree = container("root", "vertical", [
				leaf("a"),
				container("inner", "vertical", [leaf("b"), leaf("c")]),
			]);
			const result = normalize(tree) as PaneContainer;
			const sum = result.ratios.reduce((a, b) => a + b, 0);
			expect(sum).toBeCloseTo(1.0);
		});
	});

	describe("splitPane", () => {
		it("存在しない paneId の場合はツリーを変更しない", () => {
			const tree = leaf("a");
			const result = splitPane(tree, "missing", "vertical", leaf("b"));
			expect(result).toBe(tree);
		});

		it("insertBefore=true のとき対象ペインの前に挿入される", () => {
			const tree = container(
				"root",
				"vertical",
				[leaf("a"), leaf("b")],
				[0.5, 0.5],
			);
			const result = splitPane(
				tree,
				"b",
				"vertical",
				leaf("c"),
				true,
			) as PaneContainer;
			expect(result.children.map((ch) => ch.id)).toEqual(["a", "c", "b"]);
		});

		it("リーフを垂直分割 → 2ペインのコンテナ", () => {
			const tree = leaf("a");
			const newLeaf = leaf("b");
			const result = splitPane(tree, "a", "vertical", newLeaf);
			expect(result.type).toBe("container");
			const c = result as PaneContainer;
			expect(c.direction).toBe("vertical");
			expect(c.children).toHaveLength(2);
			expect(c.ratios[0]).toBeCloseTo(0.5);
			expect(c.ratios[1]).toBeCloseTo(0.5);
		});

		it("リーフを水平分割 → 2ペインのコンテナ", () => {
			const tree = leaf("a");
			const newLeaf = leaf("b");
			const result = splitPane(tree, "a", "horizontal", newLeaf);
			expect(result.type).toBe("container");
			const c = result as PaneContainer;
			expect(c.direction).toBe("horizontal");
			expect(c.children).toHaveLength(2);
		});

		it("同方向の連続分割 → フラット化で3子", () => {
			const tree = leaf("a");
			const step1 = splitPane(tree, "a", "vertical", leaf("b"));
			const step2 = splitPane(step1, "b", "vertical", leaf("c"));
			expect(step2.type).toBe("container");
			const c = step2 as PaneContainer;
			expect(c.direction).toBe("vertical");
			expect(c.children).toHaveLength(3);
			expect(c.children.map((ch) => ch.id)).toEqual(["a", "b", "c"]);
		});

		it("異方向の分割 → ネストされたコンテナ", () => {
			const tree = leaf("a");
			const step1 = splitPane(tree, "a", "vertical", leaf("b"));
			const step2 = splitPane(step1, "b", "horizontal", leaf("c"));

			expect(step2.type).toBe("container");
			const root = step2 as PaneContainer;
			expect(root.direction).toBe("vertical");
			expect(root.children).toHaveLength(2);
			expect(root.children[0].id).toBe("a");
			expect(root.children[1].type).toBe("container");
			const inner = root.children[1] as PaneContainer;
			expect(inner.direction).toBe("horizontal");
			expect(inner.children.map((ch) => ch.id)).toEqual(["b", "c"]);
		});
	});

	describe("closePane", () => {
		it("唯一のリーフを閉じるとnull", () => {
			expect(closePane(leaf("a"), "a")).toBeNull();
		});

		it("2ペインの1つを閉じると兄弟が昇格", () => {
			const tree = container("root", "vertical", [leaf("a"), leaf("b")]);
			const result = closePane(tree, "a");
			expect(result?.type).toBe("leaf");
			expect(result?.id).toBe("b");
		});

		it("3ペインの1つを閉じると2ペインのコンテナ", () => {
			const tree = container("root", "vertical", [
				leaf("a"),
				leaf("b"),
				leaf("c"),
			]);
			const result = closePane(tree, "b");
			expect(result?.type).toBe("container");
			const c = result as PaneContainer;
			expect(c.children).toHaveLength(2);
			expect(c.children.map((ch) => ch.id)).toEqual(["a", "c"]);
			const sum = c.ratios.reduce((a, b) => a + b, 0);
			expect(sum).toBeCloseTo(1.0);
		});

		it("ネストされたペインを閉じて不要なコンテナが除去される", () => {
			const tree = container("root", "vertical", [
				leaf("a"),
				container("inner", "horizontal", [leaf("b"), leaf("c")]),
			]);
			const result = closePane(tree, "b");
			// innerコンテナに1子のみ → cが昇格 → rootは[a, c]のvertical
			expect(result?.type).toBe("container");
			const c = result as PaneContainer;
			expect(c.children).toHaveLength(2);
			expect(c.children.map((ch) => ch.id)).toEqual(["a", "c"]);
		});

		it("存在しないIDでは変更なし", () => {
			const tree = container("root", "vertical", [leaf("a"), leaf("b")]);
			const result = closePane(tree, "nonexistent");
			expect(result).toBe(tree);
		});

		it("分割→閉じる→昇格の連続操作", () => {
			// リーフ→分割→3分割→1つ閉じる→2分割→もう1つ閉じる→リーフ
			let tree: PaneNode = leaf("a");
			tree = splitPane(tree, "a", "vertical", leaf("b"));
			tree = splitPane(tree, "b", "vertical", leaf("c"));
			expect(countLeaves(tree)).toBe(3);

			tree = closePane(tree, "b") as PaneNode;
			expect(countLeaves(tree)).toBe(2);

			tree = closePane(tree, "a") as PaneNode;
			expect(tree.type).toBe("leaf");
			expect(tree.id).toBe("c");
		});
	});

	describe("getAdjacentPane", () => {
		it("2ペイン垂直分割で右のペインを取得", () => {
			const tree = container("root", "vertical", [leaf("a"), leaf("b")]);
			expect(getAdjacentPane(tree, "a", "right")).toBe("b");
		});

		it("2ペイン垂直分割で左のペインを取得", () => {
			const tree = container("root", "vertical", [leaf("a"), leaf("b")]);
			expect(getAdjacentPane(tree, "b", "left")).toBe("a");
		});

		it("2ペイン水平分割で下のペインを取得", () => {
			const tree = container("root", "horizontal", [leaf("a"), leaf("b")]);
			expect(getAdjacentPane(tree, "a", "down")).toBe("b");
		});

		it("2ペイン水平分割で上のペインを取得", () => {
			const tree = container("root", "horizontal", [leaf("a"), leaf("b")]);
			expect(getAdjacentPane(tree, "b", "up")).toBe("a");
		});

		it("端のペインで範囲外方向はnull", () => {
			const tree = container("root", "vertical", [leaf("a"), leaf("b")]);
			expect(getAdjacentPane(tree, "a", "left")).toBeNull();
			expect(getAdjacentPane(tree, "b", "right")).toBeNull();
		});

		it("軸が違う方向はnull", () => {
			const tree = container("root", "vertical", [leaf("a"), leaf("b")]);
			expect(getAdjacentPane(tree, "a", "up")).toBeNull();
			expect(getAdjacentPane(tree, "a", "down")).toBeNull();
		});

		it("ネストされたツリーで隣接ペインを取得", () => {
			// [a, [b, c](horizontal)](vertical)
			const tree = container("root", "vertical", [
				leaf("a"),
				container("inner", "horizontal", [leaf("b"), leaf("c")]),
			]);
			// a → right → innerの最初のリーフ = b
			expect(getAdjacentPane(tree, "a", "right")).toBe("b");
			// b → left → a
			expect(getAdjacentPane(tree, "b", "left")).toBe("a");
			// b → down → c
			expect(getAdjacentPane(tree, "b", "down")).toBe("c");
			// c → up → b
			expect(getAdjacentPane(tree, "c", "up")).toBe("b");
		});

		it("3ペインフラットで中央から左右に移動", () => {
			const tree = container("root", "vertical", [
				leaf("a"),
				leaf("b"),
				leaf("c"),
			]);
			expect(getAdjacentPane(tree, "b", "left")).toBe("a");
			expect(getAdjacentPane(tree, "b", "right")).toBe("c");
		});

		it("リーフ単体ではnull", () => {
			const tree = leaf("a");
			expect(getAdjacentPane(tree, "a", "right")).toBeNull();
		});
	});
});
