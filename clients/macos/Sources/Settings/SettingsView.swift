import SwiftUI

struct SettingsView: View {
  var model: AppModel
  @State private var selected = Tab.general
  @State private var repositories = RepositoriesSettingsState()
  @State private var providers = ProvidersSettingsState()

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
            CardLabel(title: tab.rawValue, image: tab.icon)
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
        case .repositories: RepositoriesSettingsView(model: model, state: repositories)
        case .providers: ProvidersSettingsView(model: model, state: providers)
        case .workflows: WorkflowsSettingsView(model: model)
        case .system: SystemSettingsView(model: model)
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
    .frame(width: UIStyle.createDialogWidth, height: UIStyle.createDialogHeight)
  }
}
