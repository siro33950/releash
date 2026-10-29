import SwiftUI

struct SettingsView: View {
  var model: AppModel

  var body: some View {
    TabView {
      GeneralSettingsView(model: model)
        .tabItem { Label("General", systemImage: "gearshape") }
      RepositoriesSettingsView()
        .tabItem { Label("Repositories", systemImage: "folder") }
      ProvidersSettingsView()
        .tabItem { Label("Providers", systemImage: "terminal") }
      SystemSettingsView(model: model)
        .tabItem { Label("System", systemImage: "desktopcomputer") }
    }
    .frame(width: UIStyle.createDialogWidth, height: UIStyle.createDialogHeight)
  }
}
