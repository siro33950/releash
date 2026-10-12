import SwiftUI

struct RightPanelView: View {
  @Binding var expanded: Bool
  @State private var selected = Panel.files
  var didAppear: ((Self) -> Void)?
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
            Button {
              selected = panel
            } label: {
              Image(systemName: panel.icon)
                .frame(width: UIStyle.panelButtonSize, height: UIStyle.panelButtonSize)
                .background(
                  selected == panel ? Color(nsColor: .quaternaryLabelColor) : .clear,
                  in: RoundedRectangle(cornerRadius: UIStyle.panelButtonCornerRadius))
            }
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
        .help(expanded ? "右パネルを閉じる" : "右パネルを開く")
        .accessibilityLabel(expanded ? "右パネルを閉じる" : "右パネルを開く")
      }
      .buttonStyle(.borderless)
      .padding(.horizontal, UIStyle.spacing)
      .frame(height: UIStyle.tabBarHeight)
      Divider()
      if expanded {
        Text(selected.title).font(.headline)
          .frame(maxWidth: .infinity, alignment: .leading)
          .padding(UIStyle.padding)
          .accessibilityIdentifier("right-panel-title")
      }
      Spacer(minLength: 0)
    }
    .frame(
      minWidth: expanded ? UIStyle.rightPanelMinimumWidth : UIStyle.collapsedPanelWidth,
      idealWidth: expanded ? UIStyle.sidebarWidth : UIStyle.collapsedPanelWidth,
      maxWidth: expanded ? UIStyle.sidebarMaximumWidth : UIStyle.collapsedPanelWidth
    )
    .onAppear { didAppear?(self) }
    .frame(maxHeight: .infinity)
    .background(Color(nsColor: .controlBackgroundColor))
  }
}
