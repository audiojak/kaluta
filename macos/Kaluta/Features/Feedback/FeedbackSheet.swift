import SwiftUI

/// Help › Report a Bug… and Suggest a Feature… (spec §17.1): everything
/// the email will carry is on the sheet before it is sent.
struct FeedbackSheet: View {
    @Environment(AppModel.self) private var model
    @Bindable var draft: FeedbackDraft
    @FocusState private var titleFocused: Bool
    @State private var showsDetails = false

    static let width: CGFloat = 560
    static let pictureHeight: CGFloat = 240
    static let textHeight: CGFloat = 96

    var body: some View {
        Dialog(title: draft.kind.sheetTitle, message: message, width: Self.width) {
            if let reason = draft.cannotSend {
                Label(reason, systemImage: "exclamationmark.triangle")
                    .font(TypeRole.meta)
                    .foregroundStyle(Tone.caution)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error = draft.error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(TypeRole.meta)
                    .foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
            }
            TextField(draft.kind.titlePrompt, text: $draft.title)
                .textFieldStyle(.roundedBorder)
                .focused($titleFocused)
            TextField(draft.kind.descriptionPrompt, text: $draft.details, axis: .vertical)
                .lineLimit(4...8)
                .textFieldStyle(.roundedBorder)
            picture
            details
        } buttons: {
            if draft.sending {
                ProgressView().controlSize(.small)
                Text("Sending…").font(TypeRole.meta).foregroundStyle(.secondary)
            }
            CancelButton(help: "Close without sending (Esc)") { model.feedbackDraft = nil }
            Button("Send") { Task { await model.sendFeedback(draft) } }
                .keyboardShortcut(.defaultAction)
                .disabled(!draft.canSend)
                .hoverHelp(draft.cannotSend ?? "Send it as an email from your account (Return)")
        }
        .onAppear { titleFocused = true }
    }

    private var message: String {
        let from = draft.sender.map { " from \($0)" } ?? ""
        return "This is sent as an email\(from). A maintainer reads it before any of it is public, and your address is never published."
    }

    @ViewBuilder private var picture: some View {
        if let image = draft.picture {
            VStack(alignment: .leading, spacing: Space.s) {
                HStack(spacing: Space.m) {
                    Toggle("Include this picture of the window", isOn: $draft.includePicture)
                        .hoverHelp("Attach the picture below to the email")
                    Spacer(minLength: 0)
                    if draft.includePicture, !draft.covers.isEmpty {
                        Button("Remove Covers") { draft.covers = [] }
                            .hoverHelp("Show the whole picture again")
                    }
                }
                if draft.includePicture {
                    PictureCovers(image: image, covers: $draft.covers)
                        .frame(maxWidth: .infinity, maxHeight: Self.pictureHeight)
                    Text("Drag over anything private to cover it. Covered parts are painted out of the picture that is sent. If it is published, anyone can see it.")
                        .font(TypeRole.meta)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .dropDestination(for: URL.self) { urls, _ in
                urls.first(where: \.isFileURL).map(draft.usePicture(at:)) ?? false
            }
        } else {
            Text("No picture of the window. Drop a picture here to attach one.")
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .dropDestination(for: URL.self) { urls, _ in
                    urls.first(where: \.isFileURL).map(draft.usePicture(at:)) ?? false
                }
        }
    }

    @ViewBuilder private var details: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            if draft.kind == .bug {
                Toggle(draft.log.isEmpty ? "Include recent warnings from the log (there are none)"
                                         : "Include recent warnings from the log",
                       isOn: $draft.includeLog)
                    .disabled(draft.log.isEmpty)
                    .hoverHelp("Add Kaluta's newest warnings and errors, with addresses and keys removed")
            }
            DisclosureGroup("What else is sent", isExpanded: $showsDetails) {
                ScrollView {
                    Text(sentText)
                        .font(TypeRole.codeCaption)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(height: Self.textHeight)
            }
            .hoverHelp("The lines Kaluta adds to your email")
        }
    }

    /// The blocks added under the description, as the email carries them.
    private var sentText: String {
        FeedbackReport.body(description: "", context: draft.context, log: draft.includeLog ? draft.log : "")
    }
}

/// The picture, with the areas the user dragged over painted black, as
/// they will be in the file that is sent.
struct PictureCovers: View {
    let image: CGImage
    @Binding var covers: [CGRect]
    @State private var dragging: CGRect?

    var body: some View {
        Image(decorative: image, scale: 1)
            .resizable()
            .aspectRatio(contentMode: .fit)
            .overlay {
                GeometryReader { geometry in
                    let size = geometry.size
                    ZStack(alignment: .topLeading) {
                        ForEach(Array((covers + [dragging].compactMap { $0 }).enumerated()), id: \.offset) { _, cover in
                            Rectangle()
                                .fill(Color.black)
                                .frame(width: cover.width * size.width, height: cover.height * size.height)
                                .offset(x: cover.minX * size.width, y: cover.minY * size.height)
                        }
                    }
                    .frame(width: size.width, height: size.height, alignment: .topLeading)
                    .contentShape(Rectangle())
                    .gesture(
                        DragGesture(minimumDistance: 2)
                            .onChanged { dragging = Self.cover(from: $0.startLocation, to: $0.location, in: size) }
                            .onEnded { value in
                                if let cover = Self.cover(from: value.startLocation, to: value.location, in: size) {
                                    covers.append(cover)
                                }
                                dragging = nil
                            })
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: Radius.control))
            .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(.separator))
            .accessibilityLabel("The picture of the window that will be sent")
            .hoverHelp("Drag over a part of the picture to cover it")
    }

    /// The dragged area as fractions of the picture, kept inside it; nil
    /// when it is too small to have been meant.
    static func cover(from start: CGPoint, to end: CGPoint, in size: CGSize) -> CGRect? {
        guard size.width > 0, size.height > 0 else { return nil }
        let dragged = CGRect(x: min(start.x, end.x), y: min(start.y, end.y),
                             width: abs(end.x - start.x), height: abs(end.y - start.y))
            .intersection(CGRect(origin: .zero, size: size))
        guard dragged.width >= 3, dragged.height >= 3 else { return nil }
        return CGRect(x: dragged.minX / size.width, y: dragged.minY / size.height,
                      width: dragged.width / size.width, height: dragged.height / size.height)
    }
}
