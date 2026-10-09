import AppKit
import SwiftUI

/// An account's picture, or its initials on a colour derived from the
/// address so each account keeps the same colour everywhere (spec §7.7).
struct AccountAvatar: View {
    let account: AccountSummary
    var size: CGFloat = 24

    var body: some View {
        Group {
            if let path = account.avatarPath, let image = NSImage(contentsOfFile: path) {
                Image(nsImage: image).resizable().scaledToFill()
            } else {
                ZStack {
                    Circle().fill(AccountAvatar.color(for: account.email))
                    if account.kind == .archive {
                        Image(systemName: "archivebox.fill")
                            .font(.system(size: size * 0.45, weight: .semibold))
                            .foregroundStyle(.white)
                    } else if account.kind == .agent {
                        // An agent's mailbox (spec §7.9).
                        Image(systemName: "sparkles")
                            .font(.system(size: size * 0.45, weight: .semibold))
                            .foregroundStyle(.white)
                    } else {
                        Text(AccountAvatar.initials(name: account.displayName, email: account.email))
                            .font(.system(size: size * 0.42, weight: .semibold))
                            .foregroundStyle(.white)
                    }
                }
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .accessibilityHidden(true)
    }

    /// The account's initials and colour, as the reader shows senders.
    nonisolated static func initials(name: String?, email: String) -> String {
        Avatar.initials(name: name, email: email)
    }

    static func color(for email: String) -> Color {
        Color(nsColor: NSColor(hex: Avatar.hex(for: email)) ?? .systemGray)
    }

    nonisolated static func paletteIndex(for email: String) -> Int { Avatar.paletteIndex(for: email) }
}
