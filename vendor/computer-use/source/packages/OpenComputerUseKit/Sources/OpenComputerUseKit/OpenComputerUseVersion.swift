import Foundation

public let openComputerUseVersion = "0.3.5"

public func resolvedOpenComputerUseVersion(bundle: Bundle = .main) -> String {
    if let version = bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
       !version.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        return version
    }

    return openComputerUseVersion
}
