//! Identity Home state, including recent payments and receive sharing.

use crate::model::dashpay::{DashPaySnapshot, PaymentDirection};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::qr_code_image::QrCodeImage;
use crate::ui::state::contacts_view::contact_label;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use std::sync::Arc;

/// Payments Home lists.
const RECENT_PAYMENTS: usize = 5;

/// One of Home's recent payments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentPreview {
    /// The contact's label, or a shortened identity ID for someone who is not
    /// a contact.
    pub counterparty: String,
    pub amount_duffs: u64,
    pub incoming: bool,
}

/// Cached Home content for the currently selected identity.
#[derive(Default)]
pub struct HomeState {
    pub dismissed_checklist: bool,
    pub receive_open: bool,
    pub receive_qr: Option<QrCodeImage>,
    /// The newest payments, newest first.
    pub payments: Vec<PaymentPreview>,
    /// The snapshot `payments` were built from.
    applied: Option<Arc<DashPaySnapshot>>,
    /// The identity this content belongs to, once Home has rendered it.
    owner: Option<(Network, Identifier)>,
}

impl HomeState {
    /// Bind the content to the identity on screen. The selection can change
    /// without the hub hearing of it, so Home calls this before it renders;
    /// content of another identity is dropped, the same identity keeps its own.
    pub fn reconcile_owner(&mut self, identity: &QualifiedIdentity) {
        let owner = Some((identity.network, identity.identity.id()));
        if self.owner != owner {
            *self = Self {
                owner,
                ..Self::default()
            };
        }
    }

    /// Rebuild the recent payments from the owner's DashPay snapshot when it
    /// is not the one they were built from.
    pub fn apply_snapshot(&mut self, snapshot: Option<Arc<DashPaySnapshot>>) {
        let Some(snapshot) = snapshot else {
            return;
        };
        if self
            .applied
            .as_ref()
            .is_some_and(|applied| Arc::ptr_eq(applied, &snapshot))
        {
            return;
        }
        self.payments = recent_payments(&snapshot);
        self.applied = Some(snapshot);
    }
}

/// The newest payments of `snapshot`, labelled with the contact paid or
/// paying.
fn recent_payments(snapshot: &DashPaySnapshot) -> Vec<PaymentPreview> {
    snapshot
        .payments
        .iter()
        .take(RECENT_PAYMENTS)
        .map(|payment| {
            let incoming = payment.payment_type == PaymentDirection::Received;
            let counterparty_bytes = if incoming {
                &payment.from_identity_id
            } else {
                &payment.to_identity_id
            };
            let counterparty = Identifier::from_bytes(counterparty_bytes).ok();
            let counterparty = counterparty
                .and_then(|id| snapshot.contacts.iter().find(|c| c.identity_id == id))
                .map(contact_label)
                .unwrap_or_else(|| {
                    let id = counterparty
                        .map(|id| id.to_string(Encoding::Base58))
                        .unwrap_or_default();
                    crate::model::identity_name::shorten_id(&id)
                });
            PaymentPreview {
                counterparty,
                amount_duffs: u64::try_from(payment.amount).unwrap_or_default(),
                incoming,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dashpay::{ContactData, PaymentStatus, StoredPayment};

    fn id(byte: u8) -> Identifier {
        Identifier::from([byte; 32])
    }

    fn payment(from: u8, to: u8, amount: i64, direction: PaymentDirection) -> StoredPayment {
        StoredPayment {
            id: 0,
            tx_id: format!("tx{from}{to}{amount}"),
            from_identity_id: id(from).to_vec(),
            to_identity_id: id(to).to_vec(),
            amount,
            memo: None,
            payment_type: direction,
            status: PaymentStatus::Confirmed,
            created_at: 0,
            confirmed_at: None,
        }
    }

    #[test]
    fn payments_name_the_contact_and_shorten_anyone_else() {
        let snapshot = DashPaySnapshot {
            contacts: vec![ContactData {
                identity_id: id(2),
                nickname: Some("Bao".into()),
                note: None,
                is_hidden: false,
                account_reference: 0,
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            }],
            payments: vec![
                payment(2, 1, 5_000, PaymentDirection::Received),
                payment(1, 3, 7_000, PaymentDirection::Sent),
            ],
            ..Default::default()
        };

        let rows = recent_payments(&snapshot);

        assert_eq!(
            rows[0],
            PaymentPreview {
                counterparty: "Bao".into(),
                amount_duffs: 5_000,
                incoming: true,
            }
        );
        assert_eq!(
            rows[1].counterparty,
            crate::model::identity_name::shorten_id(&id(3).to_string(Encoding::Base58))
        );
        assert!(!rows[1].incoming);
    }

    #[test]
    fn home_lists_only_the_newest_payments() {
        let snapshot = DashPaySnapshot {
            payments: (0..8)
                .map(|n| payment(2, 1, n, PaymentDirection::Received))
                .collect(),
            ..Default::default()
        };
        assert_eq!(recent_payments(&snapshot).len(), RECENT_PAYMENTS);
    }
}
