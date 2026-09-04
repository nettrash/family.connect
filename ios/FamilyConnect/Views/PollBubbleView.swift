//
//  PollBubbleView.swift
//  FamilyConnect
//
//  A poll, inside a bubble. The QUESTION is not drawn here — it is the
//  message body, which the balloon already draws (docs/protocol.md,
//  "Polls"); this is everything underneath it.
//
//  Platform-free on purpose, for the reason LocationAttachmentView and
//  StagedAttachment are: iOS and macOS draw exactly this, from exactly the
//  same stored state. There are no bytes to fetch — a poll's whole content
//  rides on the message — so it draws immediately and works offline for
//  anything already cached.
//
//  TWO LAYOUT RULES, both learned the hard way in this repo and both
//  load-bearing here:
//
//  1. A Shape with a width-only frame is INFINITELY FLEXIBLE IN HEIGHT,
//     and one such child flips the balloon's VStack out of "everybody gets
//     their ideal height" into height DISTRIBUTION — which truncates a
//     sibling Text with an ellipsis rather than wrapping it. Every bar
//     below therefore pins its height, and every Text keeps
//     `.fixedSize(horizontal: false, vertical: true)`.
//  2. An interactive child must not swallow the balloon's own gestures.
//     A bare single-tap handler on a child masks the balloon's
//     double-tap/double-click outright (the quick heart), so each option
//     takes `count: 2` BEFORE `count: 1` — that ordering is what makes
//     them exclusive — and passes the double up. It masks the balloon's
//     LONG PRESS too, which on the phone is the only door to the reaction
//     capsule and to Reply / Copy / Share / Retry: with the options and
//     the faces covering most of the balloon, that door was shut over
//     nearly all of a poll, and holding a finger down there cast a vote on
//     release instead. So every row also carries a `simultaneousGesture`
//     that hands the press back up — the fix AttachmentView and
//     LinkPreviewCard already carry, and what Android's `combinedClickable`
//     does for the same two rows. A bare gesture publishes no accessibility
//     action either, which is why every row spells one out.
//
//  WHO VOTED. Faces are capped, so on an option most of the family chose
//  the later voters were visible only as "+N" — a count, on the one
//  feature whose whole point is that votes are not secret. Tapping the
//  faces opens the full list, which is what a reaction chip's long press
//  already does for reactions; the surface is a popover for the same
//  reason (MessageBubbleView on the phone, MacMessageRow on the Mac).
//  It is a tap rather than a long press because the faces have no other
//  action to be told apart from, and it takes `count: 2` first like
//  everything else here, so the balloon's quick heart still fires.
//

import SwiftUI

struct PollBubbleView: View {
    let poll: PollSnapshot
    /// Who is reading: decides which option is drawn as theirs, and what a
    /// tap on it means.
    let currentUserID: Int64
    /// True when this reader wrote the message — the only person who may
    /// close it (the family owner does not outrank an author here, exactly
    /// as with editing).
    var isAuthor: Bool = false
    /// Votes cannot be cast until the message has a server id, so a
    /// just-sent poll is drawn but inert.
    var isVotable: Bool = true
    /// How many people COULD vote, for the footer. 0 hides the "of N".
    var memberCount: Int = 0
    /// userID → display name, for the faces beside an option.
    var memberNames: [Int64: String] = [:]
    /// userID → profile-picture version; absent just means initials.
    var avatarVersions: [Int64: Int64] = [:]
    /// Drawn on the tinted own-message balloon, where .primary is black.
    var isMine: Bool = false
    /// Everyone this reader has blocked. Filters the FACES and the "+N"
    /// overflow and nothing else — the count beside the option, the bar and
    /// the "N of M voted" footer all go on counting a blocked member's
    /// vote (protocol.md, "Blocking a member").
    var blockedUserIDs: Set<Int64> = []
    var onVote: (Int64) -> Void = { _ in }
    var onClose: () -> Void = {}
    /// The balloon's own double tap (the quick heart), which a child must
    /// pass up rather than swallow.
    var onDoubleTap: () -> Void = {}
    /// The balloon's own long press — the reaction capsule and the message
    /// menu — which a child must pass up for the same reason. See rule 2.
    var onLongPress: () -> Void = {}

    /// The option whose voter list is up, if any. One at a time, and held
    /// by id rather than by index so a poll re-stamped while the list is
    /// open (somebody else voting) cannot switch it to another option.
    @State private var votersShownOptionID: Int64?

    /// Faces per option, capped: a family is small, but a poll with one
    /// option everybody picked would otherwise run the row off the bubble.
    /// Past the cap the row says "+N", and both it and the faces open the
    /// list of everyone.
    private static let maxFaces = 5
    private static let faceSize: CGFloat = 18
    /// The bar's height, PINNED — see rule 1 in the file header.
    private static let barHeight: CGFloat = 6

    private var myOptionID: Int64? {
        PollPresentation.myOptionID(in: poll, currentUserID: currentUserID)
    }

    private var voterCount: Int {
        PollPresentation.voterIDs(in: poll).count
    }

    /// Everything drawn on the balloon takes its colour: white on my
    /// tinted balloon, primary otherwise — the rule both message rows
    /// already follow.
    private var contentColor: Color {
        isMine ? .white : .primary
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(poll.options) { option in
                optionRow(option)
            }
            footer
        }
        // A poll decides how wide the balloon is — capped, so a two-word
        // question does not produce a full-width slab, and floored, so the
        // bars mean something. The body Text above wraps against it.
        .frame(maxWidth: 280, alignment: .leading)
        .padding(.top, 6)
    }

    @ViewBuilder
    private func optionRow(_ option: PollOptionSnapshot) -> some View {
        let isMyChoice = myOptionID == option.id
        let fraction = PollPresentation.fraction(of: option, in: poll)
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                // The check is what says "this one is yours" without
                // colour alone carrying it — the bar's tint would be the
                // only signal otherwise, and it is not enough on a
                // balloon that is already tinted.
                Image(systemName: isMyChoice ? "checkmark.circle.fill" : "circle")
                    .font(.caption)
                    .foregroundStyle(contentColor.opacity(isMyChoice ? 1 : 0.45))
                Text(option.text)
                    .font(.callout)
                    // See rule 1: the bar below is a Shape, and this is
                    // the Text it would truncate. `horizontal: false` is
                    // load-bearing — it must not touch width.
                    .fixedSize(horizontal: false, vertical: true)
                    .foregroundStyle(contentColor)
                Spacer(minLength: 4)
                // NOT filtered, deliberately: this is a TALLY, and
                // "a poll keeps its tallies and its bars" — a number that
                // moved when you blocked somebody would tell them so
                // (protocol.md, "What is NOT hidden, and why").
                Text("\(option.votes.count)")
                    .font(.caption.weight(.medium).monospacedDigit())
                    .foregroundStyle(contentColor.opacity(0.75))
                    .contentTransition(.numericText())
            }
            bar(fraction: fraction, isMyChoice: isMyChoice)
            // On the DRAWABLE list, not the raw one: the faces row carries
            // a pinned height and a leading inset, so gating on
            // `option.votes` would reserve an empty band under exactly the
            // options only a blocked member chose — positional information
            // about which one they picked, and a hole that reads as a bug.
            if !PollPresentation.drawableVoters(of: option, blockedUserIDs: blockedUserIDs).isEmpty {
                faces(of: option)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        // Count 2 BEFORE count 1 — that is what makes them exclusive. A
        // bare single-tap child otherwise masks the balloon's double tap
        // and the quick heart dies over the whole poll.
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture { vote(option) }
        // And the long press, which the taps above would otherwise eat.
        // Simultaneous rather than exclusive: it must not compete with
        // them, only travel alongside.
        .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress() })
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(accessibilityLabel(option))
        // A bare gesture publishes NO accessibility action — measured in
        // this repo, on this exact pattern. Without this VoiceOver
        // announces a button that does nothing.
        .accessibilityAction { vote(option) }
        // The faces are INSIDE this element — the row combines its
        // children — so their tap is unreachable on its own and the way
        // to the list has to be published here, as a named action. The
        // reaction chips spell out the identical pair.
        .accessibilityActions {
            if !PollPresentation.drawableVoters(of: option, blockedUserIDs: blockedUserIDs).isEmpty {
                Button("See who voted") { votersShownOptionID = option.id }
            }
        }
    }

    /// The proportional bar: a pinned-height track with a fraction of it
    /// filled. The GeometryReader lives inside an OVERLAY of an
    /// already-sized shape, so it measures without proposing anything —
    /// a bare GeometryReader here would be greedy in both directions.
    private func bar(fraction: Double, isMyChoice: Bool) -> some View {
        Capsule()
            .fill(contentColor.opacity(0.15))
            .frame(height: Self.barHeight)
            .overlay(alignment: .leading) {
                GeometryReader { geometry in
                    Capsule()
                        .fill(isMyChoice ? AnyShapeStyle(contentColor) : AnyShapeStyle(contentColor.opacity(0.45)))
                        .frame(
                            width: max(0, geometry.size.width * fraction),
                            height: Self.barHeight)
                }
            }
            .animation(.easeOut(duration: 0.25), value: fraction)
    }

    /// Who chose this one. Faces rather than names: a family is small
    /// enough to recognise, and names would wrap the row.
    ///
    /// Tapping anywhere along the row opens the full list — the "+N" is
    /// where the cap starts to hide people, but the faces are the bigger
    /// target and mean the same thing.
    private func faces(of option: PollOptionSnapshot) -> some View {
        let drawable = PollPresentation.drawableVoters(of: option, blockedUserIDs: blockedUserIDs)
        return HStack(spacing: -4) {
            ForEach(drawable.prefix(Self.maxFaces), id: \.self) { userID in
                InitialsAvatar(
                    title: name(of: userID),
                    userID: userID,
                    avatarVersion: avatarVersions[userID] ?? 0,
                    size: Self.faceSize)
            }
            // From `drawable` too. The "+N" is the overflow marker of the
            // FACES row, not a tally: computed from the raw list it would
            // print "+3" beside two visible faces — a per-option readout of
            // exactly how many blocked people chose that option, which is
            // worse than the faces it replaced.
            if drawable.count > Self.maxFaces {
                Text("+\(drawable.count - Self.maxFaces)")
                    .font(.caption2)
                    .foregroundStyle(contentColor.opacity(0.7))
                    .padding(.leading, 6)
            }
        }
        // Pinned, for the reason the bar is: a row of circles beside a
        // flexible sibling must not become the thing that stretches.
        .frame(height: Self.faceSize)
        .padding(.leading, 20)
        // The circles do not fill the row they sit in, and the gap
        // between a face and the "+N" is not a hole in the target.
        .contentShape(Rectangle())
        // Rule 2, again and for the same reason: the double goes up to
        // the balloon (the Mac's double-click quick heart included), and
        // ONLY because it is declared first.
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture { votersShownOptionID = option.id }
        // The long press goes up too — see rule 2. The faces sit INSIDE
        // the option row, so without this the capsule is unreachable over
        // the busiest part of the poll.
        .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress() })
        // A tooltip on the Mac, where a pointer can hover and nothing
        // else says what this row does.
        .help("See who voted")
        .popover(isPresented: Binding(
            get: { votersShownOptionID == option.id },
            set: { shown in
                // Only ever clears ITS OWN option: a dismissal arriving
                // after another row opened must not close that one.
                if !shown && votersShownOptionID == option.id { votersShownOptionID = nil }
            })) {
            voterList(of: option)
        }
    }

    /// Everyone who chose one option, named — the half the faces cannot
    /// show once there are more voters than there are places for them.
    ///
    /// Colours are spelled out rather than inherited: everything else here
    /// draws in `contentColor`, which is WHITE on my own tinted balloon,
    /// and a popover puts that on the system's own surface.
    private func voterList(of option: PollOptionSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(option.text)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.primary)
                .fixedSize(horizontal: false, vertical: true)
            // In the server's cast order, which is the order the faces
            // are in — the list is the same row, continued.
            ForEach(
                PollPresentation.drawableVoters(of: option, blockedUserIDs: blockedUserIDs),
                id: \.self
            ) { userID in
                HStack(spacing: 8) {
                    InitialsAvatar(
                        title: name(of: userID),
                        userID: userID,
                        avatarVersion: avatarVersions[userID] ?? 0,
                        size: 24)
                    Text(name(of: userID))
                        .font(.footnote)
                        .foregroundStyle(.primary)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .padding(12)
        // Capped, or a 100-character answer (the protocol's limit) makes a
        // popover as wide as the screen. Wrapping is what the fixedSize on
        // the header is for.
        .frame(maxWidth: 240, alignment: .leading)
        // A popover on the phone too, not the sheet a compact width would
        // otherwise adapt it into — the same call the reaction list makes.
        .presentationCompactAdaptation(.popover)
    }

    /// How many of the family have answered, and the way out for the
    /// author. A closed poll says so instead of offering the button.
    private var footer: some View {
        HStack(spacing: 8) {
            Text(footerText)
                .font(.caption2)
                .foregroundStyle(contentColor.opacity(0.7))
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            if poll.closed {
                Label("Poll closed", systemImage: "lock.fill")
                    .font(.caption2)
                    .foregroundStyle(contentColor.opacity(0.7))
            } else if isAuthor && isVotable {
                Button {
                    onClose()
                } label: {
                    Text("Close poll")
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(contentColor)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .overlay(Capsule().strokeBorder(contentColor.opacity(0.5), lineWidth: 1))
                        // Tap slack only — a negative inset, so the drawn
                        // capsule and the balloon height stay exactly as
                        // they are (heights feed the thread’s anchoring).
                        .contentShape(Capsule().inset(by: -8))
                }
                .buttonStyle(.plain)
                .accessibilityHint("Ends the poll. This cannot be undone.")
            }
        }
        .padding(.top, 2)
    }

    private var footerText: String {
        guard memberCount > 0 else {
            return String(localized: "\(voterCount) voted")
        }
        return String(localized: "\(voterCount) of \(memberCount) voted")
    }

    private func name(of userID: Int64) -> String {
        if userID == currentUserID { return String(localized: "You") }
        return memberNames[userID] ?? String(localized: "Someone")
    }

    /// Everything the eye gets from a row, in one sentence: what it says,
    /// how many chose it, and whether this reader is one of them.
    private func accessibilityLabel(_ option: PollOptionSnapshot) -> String {
        let head = String(localized: "\(option.text). \(option.votes.count) votes")
        guard myOptionID == option.id else { return head }
        return "\(head). \(String(localized: "Your choice"))"
    }

    /// A tap: the option already held clears the vote, anything else casts
    /// it. Inert on a closed poll and on one the server has never seen.
    private func vote(_ option: PollOptionSnapshot) {
        guard isVotable, !poll.closed else { return }
        onVote(option.id)
    }
}
