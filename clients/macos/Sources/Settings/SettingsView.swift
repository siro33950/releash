import SwiftUI

struct SettingsView: View {
  var model: AppModel
  @State private var selected = Tab.general

  private enum Tab: String, CaseIterable {
    case general = "General"
    case repositories = "Repositories"
    case providers = "Providers"
    case workflows = "Workflows"
    case system = "System"

    var icon: String {
      switch self {
      case .general: "settings"
      case .repositories: "folder"
      case .providers: "terminal"
      case .workflows: "waypoints"
      case .system: "monitor"
      }
    }
  }

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: UIStyle.rowSpacing) {
        ForEach(Tab.allCases, id: \.self) { tab in
          Toggle(isOn: Binding(get: { selected == tab }, set: { if $0 { selected = tab } })) {
            Label {
              Text(tab.rawValue)
            } icon: {
              Image(tab.icon).resizable().frame(
                width: UIStyle.cardIconWidth, height: UIStyle.cardIconWidth)
            }
          }
          .toggleStyle(IconToggleStyle())
          .frame(height: UIStyle.tabBarHeight)
          .accessibilityLabel(tab.rawValue)
          .accessibilityAddTraits(selected == tab ? .isSelected : [])
        }
      }
      .padding(UIStyle.spacing)
      Divider()
      Group {
        switch selected {
        case .general: GeneralSettingsView(model: model)
        case .repositories: RepositoriesSettingsView(model: model)
        case .providers: ProvidersSettingsView(model: model)
        case .workflows: WorkflowsSettingsView(model: model)
        case .system: SystemSettingsView(model: model)
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
    .frame(width: UIStyle.createDialogWidth, height: UIStyle.createDialogHeight)
  }
}
