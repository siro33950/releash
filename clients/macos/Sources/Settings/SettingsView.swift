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
    HStack(spacing: 0) {
      VStack(alignment: .leading, spacing: UIStyle.rowSpacing) {
        ForEach(Tab.allCases, id: \.self) { tab in
          Toggle(isOn: Binding(get: { selected == tab }, set: { if $0 { selected = tab } })) {
            CardLabel(title: tab.rawValue, image: tab.icon)
              .frame(maxWidth: .infinity, alignment: .leading)
              .padding(UIStyle.spacing)
          }
          .toggleStyle(IconToggleStyle())
          .frame(height: UIStyle.tabBarHeight)
          .accessibilityIdentifier("settings-navigation-\(tab.rawValue)")
          .accessibilityLabel(tab.rawValue)
          .accessibilityAddTraits(selected == tab ? .isSelected : [])
        }
        Spacer()
      }
      .padding(UIStyle.spacing)
      .frame(width: 200)
      .background(.bar)
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
    .frame(width: 1024, height: 800)
  }
}

struct SettingsLabel: View {
  let title: String
  let description: String

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      Text(title)
      Text(description)
        .font(.caption)
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)
    }
  }
}
