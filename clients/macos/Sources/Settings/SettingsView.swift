import SwiftUI

struct SettingsView: View {
  var model: AppModel

  var body: some View {
    TabView {
      GeneralSettingsView(model: model)
        .tabItem { Label("General", image: "settings") }
      RepositoriesSettingsView(model: model)
        .tabItem { Label("Repositories", image: "folder") }
      ProvidersSettingsView(model: model)
        .tabItem { Label("Providers", image: "terminal") }
      WorkflowsSettingsView(model: model)
        .tabItem { Label("Workflows", image: "waypoints") }
      SystemSettingsView(model: model)
        .tabItem { Label("System", image: "monitor") }
    }
    .frame(width: UIStyle.createDialogWidth, height: UIStyle.createDialogHeight)
  }
}
