#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

mod chain_queries;
mod chain_utils;
pub mod qp_types;
mod erc_20_client;
mod contract_client;
mod quantum_portal_client;
pub mod quantum_portal_service;

#[frame_support::pallet]
pub mod pallet {
	//! A demonstration of an offchain worker that sends onchain callbacks
	use core::{convert::TryInto};
	use parity_scale_codec::{Decode, Encode};
	use frame_support::pallet_prelude::*;
	use frame_system::{
		pallet_prelude::*,
		offchain::{
			AppCrypto,
			SignedPayload, Signer, SigningTypes,
		},
	};
	use frame_support::traits::OneSessionHandler;
	use sp_core::{crypto::KeyTypeId};
	use sp_runtime::{traits::BlockNumberProvider, RuntimeDebug, RuntimeAppPublic};
	use sp_std::{prelude::*, str};
	use serde::{Deserialize, Deserializer};
	use crate::{qp_types};
	use crate::chain_utils::{ChainUtils};
	use crate::contract_client::{ContractClient, ContractClientSignature};
	use crate::quantum_portal_client::QuantumPortalClient;
	use crate::quantum_portal_service::{PendingTransaction, QuantumPortalService};
	use crate::qp_types::{QpNetworkItem};

	#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, scale_info::TypeInfo)]
	pub struct Payload<Public> {
		number: u64,
		public: Public,
	}

	impl<T: SigningTypes> SignedPayload<T> for Payload<T::Public> {
		fn public(&self) -> T::Public {
			self.public.clone()
		}
	}

	// ref: https://serde.rs/container-attrs.html#crate
	#[derive(Deserialize, Encode, Decode, Default, RuntimeDebug, scale_info::TypeInfo)]
	struct SnapshotInfo {
		// Specify our own deserializing function to convert JSON string to vector of bytes
		#[serde(deserialize_with = "de_string_to_bytes")]
		icon_address: Vec<u8>,
		amount: u32,
		defi_user: bool,
		vesting_percentage: u32,
	}

	#[derive(Debug, Deserialize, Encode, Decode, Default)]
	struct IndexingData(Vec<u8>, u64);

	pub fn de_string_to_bytes<'de, D>(de: D) -> Result<Vec<u8>, D::Error>
		where
			D: Deserializer<'de>,
	{
		let s: &str = Deserialize::deserialize(de)?;
		Ok(s.as_bytes().to_vec())
	}

	#[pallet::config]
	pub trait Config: frame_system::offchain::CreateSignedTransaction<Call<Self>> + frame_system::Config {
		/// The overarching event type.
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
		/// The overarching dispatch call type.
		type Call: From<frame_system::Call<Self>>;
		/// The identifier type for an authority.
		type AuthorityId: Member
			+ Parameter
			+ RuntimeAppPublic
			+ MaybeSerializeDeserialize
			+ MaxEncodedLen
			+ AppCrypto<Self::Public, Self::Signature>;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	// The pallet's runtime storage items.
	// https://substrate.dev/docs/en/knowledgebase/runtime/storage
	#[pallet::storage]
	// Learn more about declaring storage items:
	// https://substrate.dev/docs/en/knowledgebase/runtime/storage#declaring-storage-i&tems
	#[pallet::getter(fn numbers)]
	pub(super) type Numbers<T> = StorageValue<_, u64, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn qp_config_item)]
	pub type QpConfigItem<T> = StorageValue<_, qp_types::QpConfig, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn pending_transactions)]
	pub(super) type PendingTransactions<T: Config> = StorageMap<_,
		Identity, u64, PendingTransaction, ValueQuery>;

	#[pallet::event]
	// #[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		NewNumber(Option<T::AccountId>, u64),
	}

	// Errors inform users that something went wrong.
	#[pallet::error]
	pub enum Error<T> {
		// Error returned when not sure which ocw function to executed
		UnknownOffchainMux,

		// Error returned when making signed transactions in off-chain worker
		NoLocalAcctForSigning,
		OffchainSignedTxError,

		// Error returned when making unsigned transactions in off-chain worker
		OffchainUnsignedTxError,

		// Error returned when making unsigned transactions with signed payloads in off-chain worker
		OffchainUnsignedTxSignedPayloadError,

		// Error returned when fetching github info
		HttpFetchingError,
		DeserializeToObjError,
		DeserializeToStrError,
	}

	#[pallet::genesis_config]
	pub struct GenesisConfig {
		pub networks: qp_types::QpConfig, 
	}

	#[cfg(feature = "std")]
	impl Default for GenesisConfig {
		fn default() -> Self {
			Self {
				networks: qp_types::QpConfig::default(),
			}
		}
	}


	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig {
		fn build(&self) {
			<QpConfigItem<T>>::put(self.networks.clone());
		}
	}

	impl<T: Config> Pallet<T> {
		pub fn configure_network(block_number: u64, network_item: QpNetworkItem
		) -> QuantumPortalClient<T> {
			let rpc_endpoint = network_item.url;
			let id = network_item.id;

			let signer = Signer::<T,
				T::AuthorityId>::any_account();
			let lgr_mgr = ChainUtils::hex_to_address(&network_item.ledger_manager[..]);
			let client = ContractClient::new(
				rpc_endpoint, &lgr_mgr, id);
			QuantumPortalClient::new(
				client,
				ContractClientSignature::from(signer),
				sp_io::offchain::timestamp().unix_millis(),
				block_number,
			)
		}
		
		pub fn test_qp(block_number: u64, qp_config_item: qp_types::QpConfig) {
			let client_vec: Vec<_> = qp_config_item.network_vec.into_iter().map(|item| Self::configure_network(block_number, item)).collect();
			let svc = QuantumPortalService::<T>::new(client_vec);
			let _res: Vec<_> = qp_config_item.pair_vec.into_iter().map(|(remote_chain, local_chain)| svc.process_pair_with_lock(remote_chain, local_chain).unwrap()).collect();
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn offchain_worker(block_number: T::BlockNumber) {
			log::info!("Hello from pallet-ocw.");
			let qp_config_item = <QpConfigItem<T>>::get();
			let bno = block_number.try_into().map_or(0 as u64, |f| f);
			Self::test_qp(bno, qp_config_item);
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {}

	impl<T: Config> BlockNumberProvider for Pallet<T> {
		type BlockNumber = T::BlockNumber;

		fn current_block_number() -> Self::BlockNumber {
			<frame_system::Pallet<T>>::block_number()
		}
	}

	impl<T: Config> sp_runtime::BoundToRuntimeAppPublic for Pallet<T> {
		type Public = T::AuthorityId;
	}
	
	impl<T: Config> OneSessionHandler<T::AccountId> for Pallet<T> {
		type Key = T::AuthorityId;
	
		fn on_genesis_session<'a, I: 'a>(_validators: I)
		where
			I: Iterator<Item = (&'a T::AccountId, T::AuthorityId)>,
		{
			// nothing to do here
		}
	
		fn on_new_session<'a, I: 'a>(_changed: bool, _validators: I, _queued_validators: I)
		where
			I: Iterator<Item = (&'a T::AccountId, T::AuthorityId)>,
		{
			// nothing to do here
		}
	
		fn on_disabled(_i: u32) {
			// nothing to do here
		}
	}
}
