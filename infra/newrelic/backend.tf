terraform {
  cloud {
    organization = "releash"

    workspaces {
      name = "newrelic"
    }
  }
}
