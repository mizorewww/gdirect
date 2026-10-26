import Foundation

struct DriveFile: Codable, Identifiable, Sendable {
    let id: String
    let name: String
    let mimeType: String
    let size: String?
    let modifiedTime: String?
    let md5Checksum: String?
    let resourceKey: String?

    // Mirrors api::File::downloadable in src/api.rs.
    var isExportable: Bool {
        ["document", "spreadsheet", "presentation", "drawing"].contains {
            mimeType == "application/vnd.google-apps.\($0)"
        }
    }

    var isDownloadable: Bool {
        !mimeType.hasPrefix("application/vnd.google-apps.") || isExportable
    }
}

struct DriveAccount: Equatable, Identifiable, Sendable {
    let email: String
    let isDefault: Bool

    var id: String { email }
}

struct TransferSettings: Sendable {
    var ip: String?
    var sni: String?
    var noSNI: Bool
    var caCert: String?
    var connections: Int
    var googleOnly: Bool

    init(
        ip: String? = nil,
        sni: String? = nil,
        noSNI: Bool = false,
        caCert: String? = nil,
        connections: Int = 8,
        googleOnly: Bool = false
    ) {
        self.ip = ip
        self.sni = sni
        self.noSNI = noSNI
        self.caCert = caCert
        self.connections = connections
        self.googleOnly = googleOnly
    }
}


enum SignInMethod: String, CaseIterable, Identifiable {
    case desktop = "Desktop OAuth client"
    case credentials = "Existing credentials"

    var id: Self { self }

    func sources(clientSecret: String, credentials: String) -> (clientSecret: String?, credentials: String?) {
        func nonEmpty(_ value: String) -> String? {
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : trimmed
        }
        switch self {
        case .desktop: return (nonEmpty(clientSecret), nil)
        case .credentials: return (nil, nonEmpty(credentials))
        }
    }
}
