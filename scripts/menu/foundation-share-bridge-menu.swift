import AppKit
import CoreGraphics
import Foundation
import ImageIO

private let bridgeAgentLabel = "com.ravonus.foundation-share-bridge"
private let menuAgentLabel = "com.ravonus.foundation-share-bridge.menu"
private let refreshInterval: TimeInterval = 15

private struct BridgeHealthResponse: Decodable {
    let status: String
    let relay_enabled: Bool
    let relay_server_url: String
    let relay_device_name: String
    let relay_device_id: String?
    let relay_device_label: String?
    let relay_last_connected_at: String?
    let relay_last_error: String?

    private enum CodingKeys: String, CodingKey {
        case status
        case relay_enabled = "relayEnabled"
        case relay_server_url = "relayServerUrl"
        case relay_device_name = "relayDeviceName"
        case relay_device_id = "relayDeviceId"
        case relay_device_label = "relayDeviceLabel"
        case relay_last_connected_at = "relayLastConnectedAt"
        case relay_last_error = "relayLastError"
    }
}

private struct BridgeConfigResponse: Decodable {
    let download_root_dir: String
    let sync_enabled: Bool
    let local_gateway_base_url: String
    let public_gateway_base_url: String
    let relay_enabled: Bool
    let relay_server_url: String
    let relay_device_name: String
    let relay_device_id: String?
    let relay_device_label: String?
    let relay_last_connected_at: String?
    let relay_last_error: String?
    let tunnel_enabled: Bool?
    let tunnel_hostname: String?
    let tunnel_last_error: String?
    let config_file: String

    private enum CodingKeys: String, CodingKey {
        case download_root_dir = "downloadRootDir"
        case sync_enabled = "syncEnabled"
        case local_gateway_base_url = "localGatewayBaseUrl"
        case public_gateway_base_url = "publicGatewayBaseUrl"
        case relay_enabled = "relayEnabled"
        case relay_server_url = "relayServerUrl"
        case relay_device_name = "relayDeviceName"
        case relay_device_id = "relayDeviceId"
        case relay_device_label = "relayDeviceLabel"
        case relay_last_connected_at = "relayLastConnectedAt"
        case relay_last_error = "relayLastError"
        case tunnel_enabled = "tunnelEnabled"
        case tunnel_hostname = "tunnelHostname"
        case tunnel_last_error = "tunnelLastError"
        case config_file = "configFile"
    }
}

private struct RuntimeEnvironment {
    let runtimeDirectory: URL
    let configFile: URL
    let bridgeBaseURL: URL
    let siteBaseURL: URL
    let lightLogoAssetFile: URL
    let darkLogoAssetFile: URL

    static func load() -> RuntimeEnvironment {
        let environment = ProcessInfo.processInfo.environment
        let homeDirectory = FileManager.default.homeDirectoryForCurrentUser
        let defaultRuntimeDirectory = homeDirectory
            .appendingPathComponent("Library")
            .appendingPathComponent("Application Support")
            .appendingPathComponent("FoundationShareBridge")
        let runtimeDirectory = URL(
            fileURLWithPath: environment["FOUNDATION_SHARE_BRIDGE_RUNTIME_DIR"] ??
                defaultRuntimeDirectory.path
        )

        let configFile = URL(
            fileURLWithPath: environment["FOUNDATION_SHARE_BRIDGE_CONFIG_FILE"] ??
                runtimeDirectory.appendingPathComponent("bridge-config.yaml").path
        )

        let bridgeBaseURL = URL(
            string: environment["FOUNDATION_SHARE_BRIDGE_LOCAL_URL"] ??
                "http://127.0.0.1:43128"
        ) ?? URL(string: "http://127.0.0.1:43128")!

        let siteBaseURL = URL(
            string: environment["FOUNDATION_SHARE_BRIDGE_SITE_URL"] ??
                "https://foundation.agorix.io"
        ) ?? URL(string: "https://foundation.agorix.io")!

        let lightLogoAssetFile = URL(
            fileURLWithPath: environment["FOUNDATION_SHARE_BRIDGE_LOGO_LIGHT_FILE"] ??
                runtimeDirectory
                .appendingPathComponent("assets")
                .appendingPathComponent("logo-light.png").path
        )

        let darkLogoAssetFile = URL(
            fileURLWithPath: environment["FOUNDATION_SHARE_BRIDGE_LOGO_DARK_FILE"] ??
                runtimeDirectory
                .appendingPathComponent("assets")
                .appendingPathComponent("logo-dark.png").path
        )

        return RuntimeEnvironment(
            runtimeDirectory: runtimeDirectory,
            configFile: configFile,
            bridgeBaseURL: bridgeBaseURL,
            siteBaseURL: siteBaseURL,
            lightLogoAssetFile: lightLogoAssetFile,
            darkLogoAssetFile: darkLogoAssetFile
        )
    }
}

final class BridgeMenuApp: NSObject, NSApplicationDelegate {
    private let runtime = RuntimeEnvironment.load()
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()

    private let bridgeStatusItem = NSMenuItem(title: "Bridge: Checking…", action: nil, keyEquivalent: "")
    private let relayStatusItem = NSMenuItem(title: "Relay: Checking…", action: nil, keyEquivalent: "")
    private let tunnelToggleItem = NSMenuItem(title: "Public Gateway: Off", action: nil, keyEquivalent: "")
    private let tunnelOpenItem = NSMenuItem(title: "Open Public Gateway", action: nil, keyEquivalent: "")
    private let tunnelCopyItem = NSMenuItem(title: "Copy Public Gateway URL", action: nil, keyEquivalent: "")

    private var refreshTimer: Timer?
    private var lastHealth: BridgeHealthResponse?
    private var lastConfig: BridgeConfigResponse?
    private lazy var lightStatusImage: NSImage = loadStatusLogoImage(
        from: runtime.lightLogoAssetFile,
        prefersDarkAppearance: false
    )
    private lazy var darkStatusImage: NSImage = loadStatusLogoImage(
        from: runtime.darkLogoAssetFile,
        prefersDarkAppearance: true
    )
    private lazy var launchAgentsDirectory: URL = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library")
        .appendingPathComponent("LaunchAgents")

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        buildMenu()
        refreshStatus(nil)
        refreshTimer = Timer.scheduledTimer(
            timeInterval: refreshInterval,
            target: self,
            selector: #selector(refreshStatus(_:)),
            userInfo: nil,
            repeats: true
        )
    }

    func applicationWillTerminate(_ notification: Notification) {
        refreshTimer?.invalidate()
    }

    private func buildMenu() {
        if let button = statusItem.button {
            button.image = currentStatusImage()
            button.toolTip = "Foundation Share Bridge"
        }

        bridgeStatusItem.isEnabled = false
        relayStatusItem.isEnabled = false

        tunnelToggleItem.target = self
        tunnelToggleItem.action = #selector(toggleTunnel)
        tunnelOpenItem.target = self
        tunnelOpenItem.action = #selector(openTunnel)
        tunnelCopyItem.target = self
        tunnelCopyItem.action = #selector(copyTunnelURL)

        menu.addItem(bridgeStatusItem)
        menu.addItem(relayStatusItem)
        menu.addItem(tunnelToggleItem)
        menu.addItem(tunnelOpenItem)
        menu.addItem(tunnelCopyItem)
        menu.addItem(.separator())
        menu.addItem(item(title: "Open Local UI", action: #selector(openLocalUI)))
        menu.addItem(item(title: "Open Archive Desktop", action: #selector(openArchiveDesktop)))
        menu.addItem(item(title: "Open Settings", action: #selector(openSettings)))
        menu.addItem(item(title: "Refresh Now", action: #selector(refreshStatus(_:))))
        menu.addItem(.separator())
        menu.addItem(item(title: "Reveal Config YAML (Advanced)", action: #selector(revealConfigFile)))
        menu.addItem(item(title: "Edit Config YAML (Advanced)", action: #selector(editConfigFile)))
        menu.addItem(item(title: "Restart Bridge", action: #selector(restartBridge)))
        menu.addItem(.separator())
        menu.addItem(item(title: "Quit Menu App", action: #selector(quitApp)))

        statusItem.menu = menu
    }

    private func item(title: String, action: Selector) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        return item
    }

    private func loadStatusLogoImage(from url: URL, prefersDarkAppearance: Bool) -> NSImage {
        if let image = colorLogoImage(from: url) {
            return image
        }

        return fallbackStatusLogoImage(prefersDarkAppearance: prefersDarkAppearance)
    }

    private func currentStatusImage() -> NSImage {
        let appearance = statusItem.button?.effectiveAppearance ?? NSApp.effectiveAppearance
        let prefersDarkAppearance = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        return prefersDarkAppearance ? darkStatusImage : lightStatusImage
    }

    private func colorLogoImage(from url: URL) -> NSImage? {
        guard FileManager.default.fileExists(atPath: url.path),
              let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let cgImage = CGImageSourceCreateImageAtIndex(source, 0, nil)
        else {
            return nil
        }

        let width = cgImage.width
        let height = cgImage.height
        let bytesPerPixel = 4
        let bytesPerRow = width * bytesPerPixel
        let colorSpace = CGColorSpaceCreateDeviceRGB()

        var sourcePixels = [UInt8](repeating: 0, count: height * bytesPerRow)
        guard let sourceContext = CGContext(
            data: &sourcePixels,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: bytesPerRow,
            space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            return nil
        }

        sourceContext.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))

        let backgroundColor = averageCornerColor(from: sourcePixels, width: width, height: height)
        let backgroundBrightness = (0.2126 * backgroundColor.red) +
            (0.7152 * backgroundColor.green) +
            (0.0722 * backgroundColor.blue)
        let backgroundDistanceThreshold: CGFloat = backgroundBrightness > 0.5 ? 0.18 : 0.12
        let backgroundBrightnessThreshold: CGFloat = backgroundBrightness > 0.5 ? 0.14 : 0.1

        var outputPixels = [UInt8](repeating: 0, count: height * bytesPerRow)
        for pixelIndex in stride(from: 0, to: sourcePixels.count, by: bytesPerPixel) {
            let red = CGFloat(sourcePixels[pixelIndex]) / 255.0
            let green = CGFloat(sourcePixels[pixelIndex + 1]) / 255.0
            let blue = CGFloat(sourcePixels[pixelIndex + 2]) / 255.0
            let alpha = CGFloat(sourcePixels[pixelIndex + 3]) / 255.0

            let brightness = (0.2126 * red) + (0.7152 * green) + (0.0722 * blue)
            let backgroundDistance = colorDistance(
                red: red,
                green: green,
                blue: blue,
                from: backgroundColor
            )
            let isBackground = alpha < 0.08 ||
                (backgroundDistance < backgroundDistanceThreshold &&
                    abs(brightness - backgroundBrightness) < backgroundBrightnessThreshold)

            outputPixels[pixelIndex] = sourcePixels[pixelIndex]
            outputPixels[pixelIndex + 1] = sourcePixels[pixelIndex + 1]
            outputPixels[pixelIndex + 2] = sourcePixels[pixelIndex + 2]
            outputPixels[pixelIndex + 3] = isBackground ? 0 : sourcePixels[pixelIndex + 3]
        }

        let outputData = Data(outputPixels)
        guard let provider = CGDataProvider(data: outputData as CFData),
              let outputImage = CGImage(
                width: width,
                height: height,
                bitsPerComponent: 8,
                bitsPerPixel: 32,
                bytesPerRow: bytesPerRow,
                space: colorSpace,
                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                provider: provider,
                decode: nil,
                shouldInterpolate: true,
                intent: .defaultIntent
              )
        else {
            return nil
        }

        let image = NSImage(cgImage: outputImage, size: NSSize(width: 18, height: 18))
        image.isTemplate = false
        return image
    }

    private func averageCornerColor(
        from pixels: [UInt8],
        width: Int,
        height: Int
    ) -> (red: CGFloat, green: CGFloat, blue: CGFloat) {
        let bytesPerPixel = 4
        let bytesPerRow = width * bytesPerPixel
        let sampleSize = max(6, min(width, height) / 14)
        let cornerOrigins = [
            (x: 0, y: 0),
            (x: max(0, width - sampleSize), y: 0),
            (x: 0, y: max(0, height - sampleSize)),
            (x: max(0, width - sampleSize), y: max(0, height - sampleSize)),
        ]

        var redTotal: CGFloat = 0
        var greenTotal: CGFloat = 0
        var blueTotal: CGFloat = 0
        var sampleCount: CGFloat = 0

        for origin in cornerOrigins {
            for y in origin.y..<(origin.y + sampleSize) {
                for x in origin.x..<(origin.x + sampleSize) {
                    let pixelIndex = (y * bytesPerRow) + (x * bytesPerPixel)
                    redTotal += CGFloat(pixels[pixelIndex]) / 255.0
                    greenTotal += CGFloat(pixels[pixelIndex + 1]) / 255.0
                    blueTotal += CGFloat(pixels[pixelIndex + 2]) / 255.0
                    sampleCount += 1
                }
            }
        }

        guard sampleCount > 0 else {
            return (red: 1, green: 1, blue: 1)
        }

        return (
            red: redTotal / sampleCount,
            green: greenTotal / sampleCount,
            blue: blueTotal / sampleCount
        )
    }

    private func colorDistance(
        red: CGFloat,
        green: CGFloat,
        blue: CGFloat,
        from background: (red: CGFloat, green: CGFloat, blue: CGFloat)
    ) -> CGFloat {
        let deltaRed = red - background.red
        let deltaGreen = green - background.green
        let deltaBlue = blue - background.blue
        return sqrt((deltaRed * deltaRed) + (deltaGreen * deltaGreen) + (deltaBlue * deltaBlue))
    }

    private func fallbackStatusLogoImage(prefersDarkAppearance: Bool) -> NSImage {
        let size = NSSize(width: 18, height: 18)
        let image = NSImage(size: size)
        image.lockFocus()

        let stroke = NSBezierPath()
        stroke.lineWidth = 1.7
        stroke.lineCapStyle = .square
        stroke.lineJoinStyle = .miter

        func point(_ x: CGFloat, _ y: CGFloat) -> NSPoint {
            NSPoint(x: x / 64.0 * size.width, y: y / 64.0 * size.height)
        }

        stroke.move(to: point(6, 18))
        stroke.line(to: point(6, 6))
        stroke.line(to: point(18, 6))

        stroke.move(to: point(58, 18))
        stroke.line(to: point(58, 6))
        stroke.line(to: point(46, 6))

        stroke.move(to: point(6, 46))
        stroke.line(to: point(6, 58))
        stroke.line(to: point(18, 58))

        stroke.move(to: point(58, 46))
        stroke.line(to: point(58, 58))
        stroke.line(to: point(46, 58))

        (prefersDarkAppearance ? NSColor(calibratedWhite: 0.96, alpha: 1) : NSColor.black).setStroke()
        stroke.stroke()

        let center = NSBezierPath()
        center.move(to: point(32, 16))
        center.curve(to: point(48, 32), controlPoint1: point(32, 24), controlPoint2: point(40, 32))
        center.curve(to: point(32, 48), controlPoint1: point(40, 32), controlPoint2: point(32, 40))
        center.curve(to: point(16, 32), controlPoint1: point(32, 40), controlPoint2: point(24, 32))
        center.curve(to: point(32, 16), controlPoint1: point(24, 32), controlPoint2: point(32, 24))
        center.close()

        NSColor(calibratedRed: 0.18, green: 0.44, blue: 0.29, alpha: 1).setFill()
        center.fill()

        image.unlockFocus()
        image.isTemplate = false
        return image
    }

    private func currentArchiveDesktopURL() -> URL {
        let base = URL(
            string: lastConfig?.relay_server_url ??
                lastHealth?.relay_server_url ??
                runtime.siteBaseURL.absoluteString
        ) ?? runtime.siteBaseURL
        return base.appendingPathComponent("desktop")
    }

    private func currentSettingsURL() -> URL {
        runtime.bridgeBaseURL.appendingPathComponent("settings")
    }

    private func currentConfigFile() -> URL {
        if let path = lastConfig?.config_file, !path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return URL(fileURLWithPath: path)
        }

        return runtime.configFile
    }

    private func updateStatusUI(bridgeOnline: Bool, relaySummary: String) {
        bridgeStatusItem.title = bridgeOnline ? "Bridge: Running" : "Bridge: Offline"
        relayStatusItem.title = "Relay: \(relaySummary)"

        guard let button = statusItem.button else { return }
        button.image = currentStatusImage()
        button.toolTip = "Foundation Share Bridge\nBridge: \(bridgeOnline ? "Running" : "Offline")\nRelay: \(relaySummary)"
    }

    private func requestJSON<T: Decodable>(
        path: String,
        method: String = "GET",
        body: Data? = nil,
        completion: @escaping (Result<T, Error>) -> Void
    ) {
        let url = runtime.bridgeBaseURL.appendingPathComponent(path)
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.httpBody = body
        if body != nil {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }

        URLSession.shared.dataTask(with: request) { data, response, error in
            if let error {
                completion(.failure(error))
                return
            }

            guard let httpResponse = response as? HTTPURLResponse else {
                completion(.failure(NSError(
                    domain: "FoundationShareBridgeMenu",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "The bridge did not return an HTTP response."]
                )))
                return
            }

            guard (200..<300).contains(httpResponse.statusCode), let data else {
                let message = data.flatMap { String(data: $0, encoding: .utf8) } ??
                    "The bridge returned status \(httpResponse.statusCode)."
                completion(.failure(NSError(
                    domain: "FoundationShareBridgeMenu",
                    code: httpResponse.statusCode,
                    userInfo: [NSLocalizedDescriptionKey: message]
                )))
                return
            }

            do {
                let decoded = try JSONDecoder().decode(T.self, from: data)
                completion(.success(decoded))
            } catch {
                completion(.failure(error))
            }
        }.resume()
    }

    @objc private func refreshStatus(_ sender: Any?) {
        requestJSON(path: "health") { (result: Result<BridgeHealthResponse, Error>) in
            DispatchQueue.main.async {
                switch result {
                case let .success(health):
                    self.lastHealth = health
                    let relaySummary: String
                    if let message = health.relay_last_error,
                       !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    {
                        relaySummary = message
                    } else if health.relay_enabled,
                              let deviceID = health.relay_device_id,
                              !deviceID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    {
                        relaySummary = "Connected"
                    } else if health.relay_enabled {
                        relaySummary = "Enabled"
                    } else {
                        relaySummary = "Not linked"
                    }

                    self.updateStatusUI(bridgeOnline: health.status == "ok", relaySummary: relaySummary)
                case let .failure(error):
                    self.lastHealth = nil
                    self.updateStatusUI(bridgeOnline: false, relaySummary: error.localizedDescription)
                }
            }
        }

        requestJSON(path: "config") { (result: Result<BridgeConfigResponse, Error>) in
            DispatchQueue.main.async {
                if case let .success(config) = result {
                    self.lastConfig = config
                    self.updateTunnelUI(from: config)
                }
            }
        }
    }

    private func updateTunnelUI(from config: BridgeConfigResponse) {
        let enabled = config.tunnel_enabled ?? false
        let hostname = config.tunnel_hostname?.trimmingCharacters(in: .whitespacesAndNewlines)
        let lastError = config.tunnel_last_error?.trimmingCharacters(in: .whitespacesAndNewlines)

        if let message = lastError, !message.isEmpty {
            tunnelToggleItem.title = "Public Gateway: \(message)"
        } else if enabled, let host = hostname, !host.isEmpty {
            tunnelToggleItem.title = "Public Gateway: \(host) ✓"
        } else if enabled {
            tunnelToggleItem.title = "Public Gateway: Starting…"
        } else {
            tunnelToggleItem.title = "Public Gateway: Off"
        }
        tunnelToggleItem.state = enabled ? .on : .off

        let hasHostname = (hostname?.isEmpty == false)
        tunnelOpenItem.isHidden = !hasHostname
        tunnelCopyItem.isHidden = !hasHostname
    }

    private func currentTunnelURL() -> URL? {
        guard let host = lastConfig?.tunnel_hostname?.trimmingCharacters(in: .whitespacesAndNewlines),
              !host.isEmpty
        else { return nil }
        return URL(string: "https://\(host)")
    }

    @objc private func toggleTunnel() {
        let current = lastConfig?.tunnel_enabled ?? false
        let body = try? JSONSerialization.data(withJSONObject: ["tunnel_enabled": !current])
        requestJSON(
            path: "config",
            method: "POST",
            body: body
        ) { (result: Result<BridgeConfigResponse, Error>) in
            DispatchQueue.main.async {
                switch result {
                case let .success(config):
                    self.lastConfig = config
                    self.updateTunnelUI(from: config)
                case let .failure(error):
                    self.showMessage(
                        title: "Public Gateway",
                        text: "Unable to toggle the public gateway: \(error.localizedDescription)"
                    )
                }
            }
        }
    }

    @objc private func openTunnel() {
        guard let url = currentTunnelURL() else { return }
        NSWorkspace.shared.open(url)
    }

    @objc private func copyTunnelURL() {
        guard let url = currentTunnelURL() else { return }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(url.absoluteString, forType: .string)
    }

    @objc private func openLocalUI() {
        NSWorkspace.shared.open(runtime.bridgeBaseURL)
    }

    @objc private func openArchiveDesktop() {
        NSWorkspace.shared.open(currentArchiveDesktopURL())
    }

    @objc private func openSettings() {
        NSWorkspace.shared.open(currentSettingsURL())
        if lastHealth == nil {
            showMessage(
                title: "Opening Settings",
                text: "The bridge settings page is opening in your browser. If it does not load, restart the bridge from this menu."
            )
        }
    }

    @objc private func revealConfigFile() {
        let configFile = currentConfigFile()
        ensureConfigDirectoryExists(for: configFile)
        if !FileManager.default.fileExists(atPath: configFile.path) {
            createDefaultConfigFile(at: configFile)
        }
        NSWorkspace.shared.activateFileViewerSelecting([configFile])
    }

    @objc private func editConfigFile() {
        let configFile = currentConfigFile()
        ensureConfigDirectoryExists(for: configFile)
        if !FileManager.default.fileExists(atPath: configFile.path) {
            createDefaultConfigFile(at: configFile)
        }
        NSWorkspace.shared.open(configFile)
    }

    private func ensureConfigDirectoryExists(for url: URL) {
        let directoryURL = url.deletingLastPathComponent()
        try? FileManager.default.createDirectory(
            at: directoryURL,
            withIntermediateDirectories: true,
            attributes: nil
        )
    }

    private func createDefaultConfigFile(at url: URL) {
        let contents = """
        download_root_dir: \(runtime.runtimeDirectory.appendingPathComponent("synced-ipfs").path)
        sync_enabled: false
        local_gateway_base_url: http://127.0.0.1:8080
        public_gateway_base_url: https://ipfs.io
        relay_enabled: false
        relay_server_url: \(runtime.siteBaseURL.absoluteString)
        relay_device_name: Foundation desktop helper
        relay_device_id: null
        relay_device_label: null
        relay_device_token: null
        relay_last_connected_at: null
        relay_last_error: null
        """

        try? contents.write(to: url, atomically: true, encoding: .utf8)
    }

    @objc private func restartBridge() {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = [
            "kickstart",
            "-k",
            "gui/\(getuid())/\(bridgeAgentLabel)"
        ]

        do {
            try process.run()
            process.waitUntilExit()
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                self.refreshStatus(nil)
            }
        } catch {
            showMessage(title: "Unable to Restart Bridge", text: error.localizedDescription)
        }
    }

    @objc private func quitApp() {
        do {
            try bootoutMenuAgent()
        } catch {
            showMessage(title: "Unable to Quit Menu App", text: error.localizedDescription)
        }
    }

    private func showMessage(title: String, text: String) {
        let alert = NSAlert()
        alert.alertStyle = .informational
        alert.messageText = title
        alert.informativeText = text
        alert.runModal()
    }

    private func bootoutMenuAgent() throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = [
            "-c",
            """
            launchctl bootout gui/\(getuid()) \(shellQuoted(menuAgentPlistURL().path)) >/dev/null 2>&1 || \
            launchctl bootout gui/\(getuid())/\(menuAgentLabel) >/dev/null 2>&1 || true
            """
        ]
        try process.run()
    }

    private func menuAgentPlistURL() -> URL {
        launchAgentsDirectory.appendingPathComponent("\(menuAgentLabel).plist")
    }

    private func shellQuoted(_ value: String) -> String {
        let escaped = value.replacingOccurrences(of: "'", with: "'\"'\"'")
        return "'\(escaped)'"
    }
}

let app = NSApplication.shared
let delegate = BridgeMenuApp()
app.delegate = delegate
app.run()
