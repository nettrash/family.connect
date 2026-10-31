//
//  SubscriptionView.swift
//  FamilyConnect
//
//  The visible half of paid families (docs/protocol.md, "Paid families",
//  2026-10-30): the subscription screen, the unpaid banner over a family's
//  chat list, the Settings row, and Manage Subscription. One definition for
//  the phone, the iPad and the Mac — they are one app with one purchase
//  (decision D1) — so the three cannot drift apart. Every decision is
//  `PaidFamilies`; the selling is `SubscriptionManager`.
//
//  The screen is Apple's `SubscriptionStoreView`: it draws each product's
//  price, period and renewal terms, which is most of what App Review checks
//  (guideline 3.1.2). What is added here is what a family gets on THIS
//  server, the privacy policy and terms of use links, and Restore Purchases
//  — ours rather than the view's, because the restored entitlement has to
//  reach the server, and the view's own button tells nobody.
//

import StoreKit
import SwiftData
import SwiftUI

// MARK: - The subscription screen

/// The products the server sells, with what a family gets. `onSubscribed`
/// runs once the server holds an ACTIVE subscription for this account —
/// after a purchase or a restore, and after the `/me` that follows it, so a
/// family created next is never raced by a `/me` from before it.
struct SubscriptionScreen: View {
    @Environment(AppSession.self) private var session
    @Environment(SubscriptionManager.self) private var subscriptions: SubscriptionManager?
    var onSubscribed: () -> Void

    @State private var errorText: String?
    @State private var isWorking = false

    private var lines: [PaidFamilies.PaywallLine] {
        PaidFamilies.paywallLines(
            limits: session.paidFamilyLimits,
            // The assistant the server reported with the family; an account
            // with no family yet has not been told, and is promised nothing.
            serverHasAssistant: AppSettings.assistantUserID != nil)
    }

    var body: some View {
        SubscriptionStoreView(productIDs: session.subscriptionProducts) {
            marketing
        }
        .storeButton(.hidden, for: .restorePurchases)
        .subscriptionStorePolicyDestination(url: PaidFamilies.privacyPolicyURL, for: .privacyPolicy)
        .subscriptionStorePolicyDestination(url: PaidFamilies.termsOfUseURL, for: .termsOfService)
        // The account's token rides on the purchase, so the server can tell
        // whose it is and Apple learns nothing about who that is.
        .inAppPurchaseOptions { [token = session.appAccountToken] _ in
            token.map { [.appAccountToken($0)] } ?? []
        }
        .onInAppPurchaseCompletion { _, result in
            await complete(result)
        }
        .safeAreaInset(edge: .bottom) {
            footer
        }
        .disabled(isWorking)
    }

    private var marketing: some View {
        VStack(spacing: 12) {
            Image(systemName: "house.fill")
                .font(.system(size: 40))
                .foregroundStyle(.tint)
                .accessibilityHidden(true)
            Text("Host Your Family")
                .font(.title2.bold())
                .multilineTextAlignment(.center)
            Text("Your family lives on this server for as long as your subscription lasts. Members join free with the family’s invite code.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 8) {
                ForEach(lines) { line in
                    // Already localized — the String overload, deliberately.
                    Label(line.text, systemImage: line.systemImage)
                }
                Label("The subscription stays with you, the family’s owner.", systemImage: "person.crop.circle.badge.checkmark")
            }
            .font(.callout)
            .frame(maxWidth: .infinity, alignment: .leading)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
    }

    private var footer: some View {
        VStack(spacing: 6) {
            if let errorText {
                Label(errorText, systemImage: "xmark.circle")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 8) {
                if isWorking { ProgressView().controlSize(.small) }
                Button("Restore Purchases") { restore() }
                    .font(.footnote)
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
        .frame(maxWidth: .infinity)
    }

    /// A purchase the view completed: the transaction, to the server. A
    /// cancelled or pending purchase says nothing — the App Store said it
    /// already, and an approved Ask to Buy arrives through the update
    /// listener.
    private func complete(_ result: Result<Product.PurchaseResult, any Error>) async {
        guard case .success(.success(let verification)) = result, let subscriptions else { return }
        errorText = nil
        isWorking = true
        defer { isWorking = false }
        show(await subscriptions.completePurchase(verification), restoring: false)
    }

    private func restore() {
        guard let subscriptions else { return }
        errorText = nil
        isWorking = true
        Task {
            defer { isWorking = false }
            show(await subscriptions.restore(), restoring: true)
        }
    }

    private func show(_ outcome: SubscriptionManager.Outcome, restoring: Bool) {
        switch outcome {
        case .accepted(let subscription) where subscription?.active == true:
            onSubscribed()
        case .accepted:
            // The server took it and holds nothing active — an expired
            // period, say. Its word for that is the refusal's.
            errorText = PaidFamilies.subscriptionRefusal(code: "subscription_invalid")
        case .refused(let code):
            errorText = PaidFamilies.subscriptionRefusal(code: code)
        case .transient:
            errorText = String(localized: "Can't reach the server. Try again.")
        case .failed:
            errorText = String(localized: "The server refused it.")
        case .nothing:
            errorText = restoring ? String(localized: "No App Store subscription was found to restore.") : nil
        }
    }
}

/// The screen as a sheet of its own, with Cancel: the gate's create, "New
/// Family…", the owner's banner and Settings all present it this way.
struct SubscriptionSheet: View {
    @Environment(\.dismiss) private var dismiss
    var onSubscribed: () -> Void = {}

    var body: some View {
        NavigationStack {
            SubscriptionScreen {
                dismiss()
                onSubscribed()
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
            }
        }
        .macSheetFrame(MacSheetSize.subscription)
    }
}

// MARK: - Manage Subscription

/// Apple's manage-subscriptions sheet on the phone and the iPad. StoreKit
/// offers none on the Mac, where the App Store's own subscriptions page
/// opens instead.
private struct ManageSubscriptionsPresenter: ViewModifier {
    @Binding var isPresented: Bool
    @Environment(\.openURL) private var openURL

    func body(content: Content) -> some View {
        #if os(iOS)
        content.manageSubscriptionsSheet(isPresented: $isPresented)
        #else
        content.onChange(of: isPresented) { _, shown in
            guard shown else { return }
            isPresented = false
            openURL(PaidFamilies.manageSubscriptionsURL)
        }
        #endif
    }
}

extension View {
    /// "Manage Subscription" — see `ManageSubscriptionsPresenter`.
    func manageSubscriptions(isPresented: Binding<Bool>) -> some View {
        modifier(ManageSubscriptionsPresenter(isPresented: isPresented))
    }
}

// MARK: - The unpaid banner

/// Over the family's chat list while it has `delete_at`: the date it will
/// be deleted, and — for its owner — Renew. Nothing at all otherwise, so the
/// list is unchanged on every family that is paid for or free.
struct FamilyPlanBanner: View {
    @Environment(AppSession.self) private var session
    @Query private var members: [MemberEntity]
    @State private var renewing = false

    /// The owner, from the roster: the one person a member's banner can
    /// point to.
    private var ownerName: String? {
        guard let familyID = session.currentFamilyID else { return nil }
        return members.first { !$0.accountDeleted && $0.familyRoles[familyID] == "owner" }?.resolvedDisplayName
    }

    var body: some View {
        if let banner = PaidFamilies.banner(
            family: session.family, isOwner: session.isOwner, ownerName: ownerName) {
            card(banner)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .sheet(isPresented: $renewing) {
                    SubscriptionSheet()
                }
        }
    }

    private func card(_ banner: PaidFamilies.Banner) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label {
                switch banner {
                case .owner(let text), .member(let text):
                    Text(text)
                }
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
            }
            .font(.subheadline.weight(.semibold))
            .fixedSize(horizontal: false, vertical: true)
            if case .owner = banner {
                HStack {
                    Spacer(minLength: 0)
                    Button("Renew") { renewing = true }
                        .buttonStyle(.borderedProminent)
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

// MARK: - Settings

/// The subscriber's row in Settings — when it renews, ends or ended — and
/// Manage Subscription; for the owner of a paid family who holds none, a
/// way to subscribe. The parent hosts the two presentations, because a
/// presentation modifier inside a Form section is not reliably presented.
struct SubscriptionSettingsSection: View {
    @Environment(AppSession.self) private var session
    @Binding var managing: Bool
    @Binding var renewing: Bool

    var body: some View {
        if PaidFamilies.showsSubscriptionRow(
            subscriptionsEnabled: session.subscriptionsEnabled, subscription: session.subscription,
            isOwner: session.isOwner, family: session.family) {
            Section {
                if let subscription = session.subscription {
                    LabeledContent("Subscription", value: PaidFamilies.subscriptionStatus(subscription))
                    if subscription.active {
                        Button("Manage Subscription") { managing = true }
                    } else {
                        Button("Renew") { renewing = true }
                    }
                } else {
                    LabeledContent("Subscription") {
                        Button("Renew") { renewing = true }
                    }
                }
            }
        }
    }
}
