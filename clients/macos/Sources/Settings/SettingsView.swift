import SwiftUI

struct SettingsView: View {
  var model: AppModel

  var body: some View {
    TabView {
      GeneralSettingsView(model: model)
        .tabItem { Label("General", systemImage: "gearshape") }
      RepositoriesSettingsView(model: model)
        .tabItem { Label("Repositories", systemImage: "folder") }
      ProvidersSettingsView(model: model)
        .tabItem { Label("Providers", systemImage: "terminal") }
      WorkflowsSettingsView(model: model)
        .tabItem { Label("Workflows", systemImage: "arrow.triangle.branch") }
      SystemSettingsView(model: model)
        .tabItem { Label("System", systemImage: "desktopcomputer") }
    }
    .frame(width: UIStyle.createDialogWidth, height: UIStyle.createDialogHeight)
  }
}
