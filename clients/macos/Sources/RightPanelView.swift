import SwiftUI

struct RightPanelView: View {
  @Binding var expanded: Bool
  var model: AppModel
  @State private var selected = Panel.files
  enum Panel: String, CaseIterable, Identifiable {
    case files, sourceControl, threads, tasks
    var id: Self { self }
    var title: String {
      switch self {
      case .files: "ファイル"
      case .sourceControl: "Source Control"
      case .threads: "Thread"
      case .tasks: "タスク"
      }
    }
    var icon: String {
      switch self {
      case .files: "doc"
      case .sourceControl: "arrow.triangle.branch"
      case .threads: "bubble.left"
      case .tasks: "checklist"
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
            .frame(width: UIStyle.iconButtonWidth, height: UIStyle.iconButtonHeight)
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
        .frame(width: UIStyle.iconButtonWidth, height: UIStyle.iconButtonHeight)
        .help(expanded ? "右パネルを閉じる" : "右パネルを開く")
        .accessibilityLabel(expanded ? "右パネルを閉じる" : "右パネルを開く")
      }
      .buttonStyle(.accessoryBar)
      .padding(.horizontal, UIStyle.spacing)
      .frame(height: UIStyle.tabBarHeight)
      Divider()
      if expanded, selected == .files, let path = model.selectedWorktree {
        FileTreeView(model: model, worktree: path).id(path)
      } else { Spacer(minLength: 0) }
    }
    .frame(
      minWidth: expanded ? UIStyle.rightPanelMinimumWidth : UIStyle.collapsedPanelWidth,
      idealWidth: expanded ? UIStyle.sidebarWidth : UIStyle.collapsedPanelWidth,
      maxWidth: expanded ? UIStyle.sidebarMaximumWidth : UIStyle.collapsedPanelWidth
    )
    .frame(maxHeight: .infinity)
  }
}
