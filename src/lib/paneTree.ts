import type {
	PaneContainer,
	PaneLeaf,
	PaneNode,
	SplitDirection,
} from "@/types/terminal-pane";

/**
 * IDでノードを検索
 */
export function findNode(tree: PaneNode, id: string): PaneNode | null {
	if (tree.id === id) return tree;
	if (tree.type === "container") {
		for (const child of tree.children) {
			const found = findNode(child, id);
			if (found) return found;
		}
	}
	return null;
}

/**
 * IDで親コンテナとそのインデックスを検索
 */
export function findParent(
	tree: PaneNode,
	id: string,
): { parent: PaneContainer; index: number } | null {
	if (tree.type === "container") {
		for (let i = 0; i < tree.children.length; i++) {
			if (tree.children[i].id === id) {
				return { parent: tree, index: i };
			}
			const found = findParent(tree.children[i], id);
			if (found) return found;
		}
	}
	return null;
}

/**
 * 全リーフをフラット取得
 */
export function getAllLeaves(tree: PaneNode): PaneLeaf[] {
	if (tree.type === "leaf") return [tree];
	return tree.children.flatMap(getAllLeaves);
}

/**
 * リーフ数を返す
 */
export function countLeaves(tree: PaneNode): number {
	if (tree.type === "leaf") return 1;
	return tree.children.reduce((sum, child) => sum + countLeaves(child), 0);
}

/**
 * 子追加時のサイズ再配分 (Hyper パターン)
 * 既存の比率を均等に縮小し、新しい子に均等な比率を割り当て
 */
export function insertRebalance(ratios: number[]): number[] {
	const newCount = ratios.length + 1;
	const newRatio = 1 / newCount;
	const scale = 1 - newRatio;
	const rebalanced = ratios.map((r) => r * scale);
	return [...rebalanced, newRatio];
}

/**
 * 子削除時のサイズ再配分 (Hyper パターン)
 * 削除された子の比率を残りの子に均等に分配
 */
export function removalRebalance(ratios: number[], index: number): number[] {
	if (ratios.length <= 1) return [];
	const removed = ratios[index];
	const remaining = ratios.filter((_, i) => i !== index);
	const totalRemaining = remaining.reduce((a, b) => a + b, 0);
	if (totalRemaining === 0) {
		const equal = 1 / remaining.length;
		return remaining.map(() => equal);
	}
	return remaining.map((r) => r + (removed * r) / totalRemaining);
}

/**
 * ツリー正規化 (Tabby パターン)
 * - 空コンテナ削除
 * - 単一子の昇格
 * - 同方向コンテナのフラット化
 */
export function normalize(tree: PaneNode): PaneNode | null {
	if (tree.type === "leaf") return tree;

	// 子を再帰的に正規化
	const normalizedChildren: PaneNode[] = [];
	const normalizedRatios: number[] = [];

	for (let i = 0; i < tree.children.length; i++) {
		const child = normalize(tree.children[i]);
		if (child === null) continue;

		// 同方向コンテナのフラット化
		if (child.type === "container" && child.direction === tree.direction) {
			const parentRatio = tree.ratios[i];
			for (let j = 0; j < child.children.length; j++) {
				normalizedChildren.push(child.children[j]);
				normalizedRatios.push(parentRatio * child.ratios[j]);
			}
		} else {
			normalizedChildren.push(child);
			normalizedRatios.push(tree.ratios[i]);
		}
	}

	// 空コンテナ削除
	if (normalizedChildren.length === 0) return null;

	// 単一子の昇格
	if (normalizedChildren.length === 1) return normalizedChildren[0];

	// 比率を正規化（合計=1.0）
	const sum = normalizedRatios.reduce((a, b) => a + b, 0);
	const finalRatios =
		sum > 0 ? normalizedRatios.map((r) => r / sum) : normalizedRatios;

	return {
		...tree,
		children: normalizedChildren,
		ratios: finalRatios,
	};
}

/**
 * ノードを置き換えたツリーを返す
 */
function replaceNode(
	tree: PaneNode,
	targetId: string,
	replacement: PaneNode,
): PaneNode {
	if (tree.id === targetId) return replacement;
	if (tree.type === "container") {
		return {
			...tree,
			children: tree.children.map((child) =>
				replaceNode(child, targetId, replacement),
			),
		};
	}
	return tree;
}

let paneContainerIdCounter = 0;
function nextContainerId(): string {
	paneContainerIdCounter += 1;
	return `pane-container-${paneContainerIdCounter}`;
}

/** テスト用: カウンタリセット */
export function _resetContainerIdCounter(): void {
	paneContainerIdCounter = 0;
}

/**
 * 指定ペインを分割 → 新ツリーを返す
 * 同方向なら親に追加、異方向なら新コンテナに変換
 */
export function splitPane(
	tree: PaneNode,
	paneId: string,
	direction: SplitDirection,
	newPane: PaneLeaf,
	insertBefore = false,
): PaneNode {
	const parentResult = findParent(tree, paneId);

	if (parentResult && parentResult.parent.direction === direction) {
		// 親が同方向 → 親の children に新リーフを追加
		const { parent, index } = parentResult;
		const newChildren = [...parent.children];
		const insertIndex = insertBefore ? index : index + 1;
		newChildren.splice(insertIndex, 0, newPane);
		// insertRebalance は末尾に追加するので、適切な位置に挿入し直す
		const baseRatios = parent.ratios.map(
			(r) => r * (1 - 1 / (parent.children.length + 1)),
		);
		const insertedRatio = 1 / (parent.children.length + 1);
		const adjustedRatios = [...baseRatios];
		adjustedRatios.splice(insertIndex, 0, insertedRatio);

		const updatedParent: PaneContainer = {
			...parent,
			children: newChildren,
			ratios: adjustedRatios,
		};

		if (tree.id === parent.id) return normalize(updatedParent) ?? tree;
		const newTree = replaceNode(tree, parent.id, updatedParent);
		return normalize(newTree) ?? tree;
	}

	// 親が異方向 or ルートリーフ → 新コンテナに変換
	const target = findNode(tree, paneId);
	if (!target) return tree;
	const children = insertBefore ? [newPane, target] : [target, newPane];
	const newContainer: PaneContainer = {
		type: "container",
		id: nextContainerId(),
		direction,
		children,
		ratios: [0.5, 0.5],
	};

	if (tree.id === paneId) return normalize(newContainer) ?? tree;
	const newTree = replaceNode(tree, paneId, newContainer);
	return normalize(newTree) ?? tree;
}

/**
 * ペイン閉じる → 新ツリーを返す（最後のリーフなら null）
 */
export function closePane(tree: PaneNode, paneId: string): PaneNode | null {
	if (tree.type === "leaf") {
		return tree.id === paneId ? null : tree;
	}

	const childIndex = tree.children.findIndex((c) => c.id === paneId);
	if (childIndex !== -1) {
		// 直接の子を削除
		const newChildren = tree.children.filter((_, i) => i !== childIndex);
		const newRatios = removalRebalance(tree.ratios, childIndex);

		if (newChildren.length === 0) return null;
		if (newChildren.length === 1) return newChildren[0];

		const updated: PaneContainer = {
			...tree,
			children: newChildren,
			ratios: newRatios,
		};
		return normalize(updated);
	}

	// 再帰的に探す
	const newChildren: PaneNode[] = [];
	const newRatios: number[] = [];
	let removed = false;

	for (let i = 0; i < tree.children.length; i++) {
		const result = closePane(tree.children[i], paneId);
		if (result === null) {
			removed = true;
			// この子は完全に削除 → removalRebalance
		} else if (result !== tree.children[i]) {
			removed = true;
			newChildren.push(result);
			newRatios.push(tree.ratios[i]);
		} else {
			newChildren.push(tree.children[i]);
			newRatios.push(tree.ratios[i]);
		}
	}

	if (!removed) return tree;
	if (newChildren.length === 0) return null;

	// 削除されたインデックスの比率を再配分
	const sum = newRatios.reduce((a, b) => a + b, 0);
	const normalizedRatios = sum > 0 ? newRatios.map((r) => r / sum) : newRatios;

	if (newChildren.length === 1) return newChildren[0];

	return normalize({
		...tree,
		children: newChildren,
		ratios: normalizedRatios,
	});
}

type NavigationDirection = "left" | "right" | "up" | "down";

/**
 * 隣接ペインID取得 (Warp アルゴリズム)
 * 1. 対象ペインから上方向に辿り、移動方向の軸と一致するコンテナを見つける
 * 2. そのコンテナ内で次/前のインデックスの子を取得
 * 3. その子から下方向に降りて最初/最後のリーフを返す
 */
export function getAdjacentPane(
	tree: PaneNode,
	paneId: string,
	direction: NavigationDirection,
): string | null {
	const path = getPathToNode(tree, paneId);
	if (!path) return null;

	const axis: SplitDirection =
		direction === "left" || direction === "right" ? "vertical" : "horizontal";
	const forward = direction === "right" || direction === "down";

	// 上方向に辿り、軸が一致するコンテナを見つける
	for (let i = path.length - 2; i >= 0; i--) {
		const ancestor = path[i];
		if (ancestor.type !== "container" || ancestor.direction !== axis) continue;

		const childInPath = path[i + 1];
		const childIndex = ancestor.children.findIndex(
			(c) => c.id === childInPath.id,
		);
		const targetIndex = forward ? childIndex + 1 : childIndex - 1;

		if (targetIndex < 0 || targetIndex >= ancestor.children.length) continue;

		const target = ancestor.children[targetIndex];
		return forward ? getFirstLeaf(target).id : getLastLeaf(target).id;
	}

	return null;
}

/**
 * ルートからノードへのパスを取得
 */
function getPathToNode(tree: PaneNode, id: string): PaneNode[] | null {
	if (tree.id === id) return [tree];
	if (tree.type === "container") {
		for (const child of tree.children) {
			const path = getPathToNode(child, id);
			if (path) return [tree, ...path];
		}
	}
	return null;
}

/**
 * サブツリーの最初のリーフを返す
 */
function getFirstLeaf(node: PaneNode): PaneLeaf {
	if (node.type === "leaf") return node;
	return getFirstLeaf(node.children[0]);
}

/**
 * サブツリーの最後のリーフを返す
 */
function getLastLeaf(node: PaneNode): PaneLeaf {
	if (node.type === "leaf") return node;
	return getLastLeaf(node.children[node.children.length - 1]);
}
