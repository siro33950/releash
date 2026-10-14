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
      case .files: "doc"
      case .sourceControl: "arrow.triangle.branch"
      case .threads: "bubble.left"
      case .roots: "list.bullet"
      }
    }
  }
  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: UIStyle.rowSpacing) {
        if expanded {
          ForEach(Panel.allCases) { panel in
            Toggle(isOn: Binding(get: { selected == panel }, set: { if $0 { selected = panel } })) {
              Image(systemName: panel.icon)
            }
            .toggleStyle(IconToggleStyle())
            .iconButtonFrame()
            .help(panel.title).accessibilityLabel(panel.title)
            .accessibilityAddTraits(selected == panel ? .isSelected : [])
          }
          Spacer(minLength: 0)
        }
        Button {
          expanded.toggle()
        } label: {
          Image(systemName: "sidebar.right")
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
        FileTreeView(model: model, worktree: path).id(path)
      } else { Spacer(minLength: 0) }
    }
    .frame(
      minWidth: expanded ? UIStyle.rightPanelMinimumWidth : UIStyle.collapsedPanelWidth,
      idealWidth: expanded ? UIStyle.centerInitialWidth : UIStyle.collapsedPanelWidth,
      maxWidth: expanded ? .infinity : UIStyle.collapsedPanelWidth
    )
    .frame(maxHeight: .infinity)
  }
}
