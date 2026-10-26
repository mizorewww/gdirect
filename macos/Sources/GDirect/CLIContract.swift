import Foundation

enum CLICommand {
    case login(clientSecret: String? = nil, credentials: String? = nil)
    case reauth
    case accounts
    case switchAccount(String)
    case logout(account: String? = nil)
    case logoutAll
    case list(folder: String = "root", ip: String? = nil, sni: String? = nil)
    case download(
        file: String,
        output: String,
        exportMIME: String? = nil,
        settings: TransferSettings = TransferSettings()
    )
    case upload(file: String, parent: String = "root", name: String? = nil, ip: String? = nil, sni: String? = nil)
    case scan(ips: String? = nil)
    case scanList
    case mount(path: String, readOnly: Bool = false, connections: Int = 8)
    case unmount

    var arguments: [String] {
        switch self {
        case let .login(clientSecret, credentials):
            var arguments = ["login"]
            if let clientSecret {
                arguments.append("--client-secret=\(clientSecret)")
            }
            if let credentials {
                arguments.append("--credentials=\(credentials)")
            }
            return arguments
        case .reauth:
            return ["login", "--reauth"]
        case .accounts:
            return ["accounts"]
        case let .switchAccount(account):
            return ["switch", "--", account]
        case let .logout(account):
            var arguments = ["logout"]
            if let account {
                arguments += ["--", account]
            }
            return arguments
        case .logoutAll:
            return ["logout", "--all"]
        case let .list(folder, ip, sni):
            var arguments = ["ls", "--json"]
            if let ip {
                arguments += ["--ip", ip]
            }
            if let sni {
                arguments.append("--sni=\(sni)")
            }
            return arguments + ["--", folder]
        case let .download(file, output, exportMIME, settings):
            var arguments = ["download", "--output=\(output)"]
            if let exportMIME {
                arguments.append("--export=\(exportMIME)")
            }
            arguments += settings.arguments
            return arguments + ["--", file]
        case let .upload(file, parent, name, ip, sni):
            var arguments = ["upload", "--parent", parent]
            if let name {
                arguments.append("--name=\(name)")
            }
            if let ip {
                arguments += ["--ip", ip]
            }
            if let sni {
                arguments.append("--sni=\(sni)")
            }
            return arguments + ["--", file]
        case let .scan(ips):
            var arguments = ["scan"]
            if let ips {
                arguments.append("--ips=\(ips)")
            }
            return arguments
        case .scanList:
            return ["scan", "--list"]
        case let .mount(path, readOnly, connections):
            var arguments = ["mount"]
            if readOnly {
                arguments.append("--read-only")
            }
            arguments += ["--connections", String(connections)]
            return arguments + ["--", path]
        case .unmount:
            return ["unmount"]
        }
    }

    func arguments(googleOnly: Bool) -> [String] {
        googleOnly ? ["--google-only"] + arguments : arguments
    }
}

private extension TransferSettings {
    var arguments: [String] {
        var arguments: [String] = []
        if let ip {
            arguments += ["--ip", ip]
        }
        if let sni, !noSNI {
            arguments.append("--sni=\(sni)")
        }
        if noSNI {
            arguments.append("--no-sni")
        }
        if let caCert {
            arguments.append("--ca-cert=\(caCert)")
        }
        arguments += ["--connections", String(connections)]
        return arguments
    }
}

enum CLILocatorError: LocalizedError {
    case bundledExecutableUnavailable(URL)

    var errorDescription: String? {
        switch self {
        case let .bundledExecutableUnavailable(url):
            "Bundled gdirect executable is missing or not executable at \(url.path)"
        }
    }
}

struct CLILocator {
    let bundle: Bundle
    let environment: [String: String]

    init(
        bundle: Bundle = .main,
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) {
        self.bundle = bundle
        self.environment = environment
    }

    func executableURL() throws -> URL {
        let url = bundle.bundleURL
            .appendingPathComponent("Contents/MacOS/gdirect", isDirectory: false)
        guard FileManager.default.isExecutableFile(atPath: url.path) else {
            throw CLILocatorError.bundledExecutableUnavailable(url)
        }
        return url
    }

    func processEnvironment() -> [String: String] {
        var result = environment
        for key in ["GDIRECT_ACCOUNT", "GDIRECT_ACCESS_TOKEN", "GDIRECT_GOOGLE_ONLY"] {
            result.removeValue(forKey: key)
        }
        var paths = ["/opt/homebrew/bin", "/usr/local/bin"]
        let inherited = environment["PATH"]?.split(separator: ":").map(String.init) ?? []
        for path in inherited where !paths.contains(path) {
            paths.append(path)
        }
        result["PATH"] = paths.joined(separator: ":")
        return result
    }
}
