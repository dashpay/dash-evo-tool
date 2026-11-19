//! Platform writes are gated before wallet access, signing or funding.

use super::*;

impl BackendTask {
    /// Whether this task can change Platform state or lock funds for it.
    pub(crate) fn changes_platform_state(&self) -> bool {
        match self {
            Self::IdentityTask(task) => match task {
                IdentityTask::RegisterIdentity(..)
                | IdentityTask::TopUpIdentity(..)
                | IdentityTask::TopUpIdentityFromPlatformAddresses { .. }
                | IdentityTask::AddKeyToIdentity(..)
                | IdentityTask::AddDerivedKeyToIdentity { .. }
                | IdentityTask::WithdrawFromIdentity(..)
                | IdentityTask::Transfer(..)
                | IdentityTask::TransferToAddresses { .. }
                | IdentityTask::RegisterDpnsName(..) => true,
                IdentityTask::LoadIdentity(..)
                | IdentityTask::SearchIdentityFromWallet(..)
                | IdentityTask::SearchIdentitiesUpToIndex(..)
                | IdentityTask::SearchIdentityByDpnsName(..)
                | IdentityTask::ProtectIdentityKeys { .. }
                | IdentityTask::UnprotectIdentityKeys { .. }
                | IdentityTask::CheckLegacyRecovery { .. }
                | IdentityTask::RecoverLegacyIdentityData { .. }
                | IdentityTask::RemoveIdentity { .. }
                | IdentityTask::RefreshIdentity(..)
                | IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames
                | IdentityTask::CheckUsernameAvailability { .. }
                | IdentityTask::RefreshMyUsernameRequests
                | IdentityTask::SetMainUsername { .. }
                | IdentityTask::DismissUsernameRequest { .. }
                | IdentityTask::MarkUsernameOutcomesSeen { .. } => false,
            },
            Self::WalletTask(task) => match task {
                WalletTask::TransferPlatformCredits { .. }
                | WalletTask::FundPlatformAddressFromAssetLock { .. }
                | WalletTask::WithdrawFromPlatformAddress { .. }
                | WalletTask::FundPlatformAddressFromWalletUtxos { .. } => true,
                WalletTask::GenerateReceiveAddress { .. }
                | WalletTask::DeriveKeyForDisplay { .. }
                | WalletTask::GeneratePlatformReceiveAddress { .. }
                | WalletTask::WarmIdentityAuthPubkeys { .. }
                | WalletTask::SignMessageWithKey { .. }
                | WalletTask::DeriveIdentityKeyForDisplay { .. }
                | WalletTask::SignMessageWithIdentityKey { .. }
                | WalletTask::FetchPlatformAddressBalances { .. }
                | WalletTask::ListTrackedAssetLocks { .. }
                | WalletTask::GetAssetLockMaxAmount { .. }
                | WalletTask::GetSpendLimit { .. }
                | WalletTask::RenameHdWallet { .. }
                | WalletTask::RenameSingleKeyWallet { .. } => false,
            },
            Self::CoreTask(task) => match task {
                CoreTask::CreateRegistrationAssetLock(..) | CoreTask::CreateTopUpAssetLock(..) => {
                    true
                }
                CoreTask::GetBestChainLocks
                | CoreTask::RefreshWalletInfo(..)
                | CoreTask::RefreshSingleKeyWalletInfo(..)
                | CoreTask::SendWalletPayment { .. }
                | CoreTask::SendSingleKeyWalletPayment { .. }
                | CoreTask::MineBlocks { .. } => false,
            },
            Self::ContestedResourceTask(task) => match task {
                ContestedResourceTask::SubmitDpnsVoteOperation { .. }
                | ContestedResourceTask::CastScheduledVote(..)
                | ContestedResourceTask::CastDueScheduledVotes { .. } => true,
                ContestedResourceTask::QueryDPNSContests
                | ContestedResourceTask::SaveDpnsVotingPreference(..)
                | ContestedResourceTask::RefreshContestsInBackground
                | ContestedResourceTask::ReconcileDpnsVoteOperation(..)
                | ContestedResourceTask::ClearAllScheduledVotes
                | ContestedResourceTask::ClearExecutedScheduledVotes
                | ContestedResourceTask::CancelScheduledDpnsVote { .. }
                | ContestedResourceTask::EditScheduledDpnsVote(..) => false,
            },
            Self::TokenTask(task) => match task.as_ref() {
                TokenTask::RegisterTokenContract { .. }
                | TokenTask::MintTokens { .. }
                | TokenTask::TransferTokens { .. }
                | TokenTask::BurnTokens { .. }
                | TokenTask::DestroyFrozenFunds { .. }
                | TokenTask::FreezeTokens { .. }
                | TokenTask::UnfreezeTokens { .. }
                | TokenTask::PauseTokens { .. }
                | TokenTask::ResumeTokens { .. }
                | TokenTask::ClaimTokens { .. }
                | TokenTask::UpdateTokenConfig { .. }
                | TokenTask::PurchaseTokens { .. }
                | TokenTask::SetDirectPurchasePrice { .. } => true,
                TokenTask::QueryMyTokenBalances
                | TokenTask::QueryIdentityTokenBalance(..)
                | TokenTask::StopTrackingTokenBalance(..)
                | TokenTask::ClearOncePerIdentityClaimHint(..)
                | TokenTask::QueryDescriptionsByKeyword(..)
                | TokenTask::FetchTokenByContractId(..)
                | TokenTask::FetchTokenByTokenId(..)
                | TokenTask::SaveTokenLocally(..)
                | TokenTask::QueryTokenPricing(..)
                | TokenTask::EstimatePerpetualTokenRewardsWithExplanation { .. } => false,
            },
            Self::DocumentTask(task) => match task.as_ref() {
                DocumentTask::BroadcastDocument { .. }
                | DocumentTask::DeleteDocument { .. }
                | DocumentTask::ReplaceDocument { .. }
                | DocumentTask::TransferDocument { .. }
                | DocumentTask::PurchaseDocument { .. }
                | DocumentTask::SetDocumentPrice { .. } => true,
                DocumentTask::FetchDocuments(..) | DocumentTask::FetchDocumentsPage(..) => false,
            },
            Self::ContractTask(task) => match task.as_ref() {
                ContractTask::RegisterDataContract(..) | ContractTask::UpdateDataContract(..) => {
                    true
                }
                ContractTask::FetchContracts(..)
                | ContractTask::FetchContractsWithDescriptions(..)
                | ContractTask::FetchActiveGroupActions(..)
                | ContractTask::RemoveContract(..)
                | ContractTask::SaveDataContract(..) => false,
            },
            Self::DashPayTask(task) => match task.as_ref() {
                DashPayTask::UpdateProfile { .. }
                | DashPayTask::SendContactRequest { .. }
                | DashPayTask::SendContactRequestWithProof { .. }
                | DashPayTask::AcceptContactRequest { .. }
                | DashPayTask::RejectContactRequest { .. }
                | DashPayTask::CancelContactRequest { .. }
                | DashPayTask::UpdateContactInfo { .. } => true,
                DashPayTask::LoadProfile { .. }
                | DashPayTask::LoadContacts { .. }
                | DashPayTask::LoadContactsOffline { .. }
                | DashPayTask::LoadContactRequests { .. }
                | DashPayTask::FetchContactProfile { .. }
                | DashPayTask::FetchAvatar { .. }
                | DashPayTask::SearchProfiles { .. }
                | DashPayTask::LoadPaymentHistory { .. }
                | DashPayTask::SendPaymentToContact { .. }
                | DashPayTask::RegisterDashPayAddresses { .. }
                | DashPayTask::DetectIncomingContactPayments { .. }
                | DashPayTask::GenerateAutoAcceptQrCode { .. } => false,
            },
            Self::ShieldedTask(_) | Self::BroadcastStateTransition(_) => true,
            Self::SystemTask(_)
            | Self::PlatformInfo(_)
            | Self::GroveSTARKTask(_)
            | Self::MigrationTask(_)
            | Self::ReinitCoreClientAndSdk
            | Self::SwitchNetwork { .. }
            | Self::DiscoverDapiNodes { .. }
            | Self::None => false,
        }
    }
}

impl AppContext {
    /// Keep operation-specific error routing on both preflight checks.
    pub(super) fn ensure_platform_task_supported(
        &self,
        task: &BackendTask,
        sdk: &dash_sdk::Sdk,
    ) -> Result<(), TaskError> {
        if task.changes_platform_state()
            && let Err(error) = self
                .ensure_platform_write_supported()
                .and_then(|()| self.ensure_sdk_write_supported(sdk))
        {
            if matches!(
                task,
                BackendTask::ContestedResourceTask(
                    ContestedResourceTask::CastDueScheduledVotes { .. }
                )
            ) {
                return Err(TaskError::ScheduledVoteSweepFailed {
                    network: self.network,
                    source: Box::new(error),
                });
            }
            if let Some((identity_id, contact_id)) = dashpay_contact_info_target(task) {
                return Err(TaskError::DashPayContactInfoActionFailed {
                    identity_id,
                    contact_id,
                    source: Box::new(error),
                });
            }
            return Err(match dashpay_request_id(task) {
                Some(request_id) => TaskError::DashPayContactRequestActionFailed {
                    request_id,
                    source: Box::new(error),
                },
                None => error,
            });
        }

        Ok(())
    }
}
