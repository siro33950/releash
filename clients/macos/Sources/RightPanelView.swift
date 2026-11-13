import SwiftUI

struct RightPanelView: View {
  @Binding var expanded: Bool
  var model: AppModel
  @State private var selected = Panel.roots
  enum Panel: String, CaseIterable, Identifiable {
    case roots, files, sourceControl, threads
    var id: Self { self }
    var title: String {
      switch self {
      case .files: "ファイル"
      case .sourceControl: "Source Control"
      case .threads: "Thread"
      case .roots: "Node"
      }
    }
    var icon: String {
      switch self {
      case .files: "file"
      case .sourceControl: "git-branch"
      case .threads: "message-square"
      case .roots: "list"
      }
    }
  }
  private var sourceControlBadge: String? {
    guard let path = model.selectedWorktree,
      let branch = model.repositories.lazy.flatMap({ $0.branches.items }).first(where: {
        $0.worktreePath == path
      }),
      branch.hasDirtyCount, !branch.hasDirtyCountError, branch.dirtyCount > 0
    else { return nil }
    return branch.dirtyCount > 99 ? "99+" : String(branch.dirtyCount)
  }

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: UIStyle.rowSpacing) {
        if expanded {
          ForEach(Panel.allCases) { panel in
            Toggle(isOn: Binding(get: { selected == panel }, set: { if $0 { selected = panel } })) {
              CardIcon(image: Image(panel.icon))
            }
            .toggleStyle(IconToggleStyle())
            .iconButtonFrame()
            .help(panel.title).accessibilityLabel(panel.title)
            .accessibilityIdentifier("right-panel:\(panel.rawValue)")
            .accessibilityAddTraits(selected == panel ? .isSelected : [])
            .overlay(alignment: .topTrailing) {
              if panel == .sourceControl, let badge = sourceControlBadge {
                Text(badge)
                  .font(.system(size: 9))
                  .foregroundStyle(.white)
                  .frame(width: 20, height: 20)
                  .background(.gray, in: Circle())
                  .offset(x: 6, y: -4)
                  .allowsHitTesting(false)
              }
            }
          }
          Spacer(minLength: 0)
        }
        Button {
          expanded.toggle()
        } label: {
          CardIcon(image: Image("panel-right"))
        }
        .buttonStyle(IconButtonStyle())
        .iconButtonFrame()
        .help(expanded ? "右パネルを閉じる" : "右パネルを開く")
        .accessibilityLabel(expanded ? "右パネルを閉じる" : "右パネルを開く")
      }
      .padding(.horizontal, UIStyle.spacing)
      .frame(height: UIStyle.tabBarHeight)
      Divider()
      if expanded, selected == .roots, let path = model.selectedWorktree {
        RootListView(model: model, path: path).id(path)
      } else if expanded, selected == .files, let path = model.selectedWorktree {
        FileWorkspaceView(model: model, worktree: path).id(path)
      } else if expanded, selected == .threads, let path = model.selectedWorktree {
        ThreadPanelView(model: model, worktree: path).id(path)
      } else {
        Spacer(minLength: 0)
      }
    }
    .frame(maxHeight: .infinity)
  }
}
