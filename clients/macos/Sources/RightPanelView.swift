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
