#![cfg_attr(not(feature = "std"), no_std)]
use crate::{
    chain_queries::CallResponse,
    chain_utils::{ChainRequestError, ChainRequestResult, ChainUtils, TransactionCreationError},
    contract_client::{ContractClient, ContractClientSignature},
    eip_712_utils::EIP712Utils,
    qp_types::{EIP712Config, QpLocalBlock, QpRemoteBlock, QpTransaction},
    Config,
};
use ethabi_nostd::{decoder::decode, ParamKind, Token};
use sp_core::{H256, U256};
use sp_std::marker::PhantomData;
use sp_std::prelude::*;

#[allow(dead_code)]
const DUMMY_HASH: H256 = H256::zero();
const ZERO_HASH: H256 = H256::zero();

pub struct QuantumPortalClient<T: Config> {
    pub contract: ContractClient,
    pub signer: ContractClientSignature,
    pub now: u64,
    pub block_number: u64,
    pub eip_712_config: EIP712Config,
    _phantom: PhantomData<T>,
}

fn local_block_tuple0() -> Vec<ParamKind> {
    vec![
        ParamKind::Uint(256),
        ParamKind::Uint(256),
        ParamKind::Uint(256),
    ]
}

fn local_block_tuple() -> ParamKind {
    ParamKind::Tuple(vec![
        Box::new(ParamKind::Uint(256)),
        Box::new(ParamKind::Uint(256)),
        Box::new(ParamKind::Uint(256)),
    ])
}

fn decode_remote_block_and_txs<T, F>(
    data: &[u8],
    mined_block_tuple: ParamKind,
    block_tuple_decoder: F,
) -> ChainRequestResult<(T, Vec<QpTransaction>)>
where
    F: Fn(Token) -> ChainRequestResult<T>,
{
    log::info!("decode_remote_block_and_txs {:?}", data);
    // let dec = decode(
    //     &[
    //         ParamKind::Tuple(vec![
    //             Box::new(mined_block_tuple),
    //             Box::new(ParamKind::Array(
    //                 Box::new(ParamKind::Tuple(vec![         // RemoteTransaction[]
    //                                                         Box::new(ParamKind::Uint(256)),
    // // timestamp
    // Box::new(ParamKind::Address),       // remoteContract
    // Box::new(ParamKind::Address),       // sourceMsgSender
    // Box::new(ParamKind::Address),       // sourceBeneficiary
    // Box::new(ParamKind::Address),       // token
    // Box::new(ParamKind::Uint(256)),     // amount
    // Box::new(ParamKind::Bytes),         // method
    // Box::new(ParamKind::Uint(256)),     // gas                 ]))))
    //         ])],
    //     ChainUtils::hex_to_bytes(&data)?.as_slice(),
    // ).unwrap();
    let dec = decode(
        &[
            mined_block_tuple,
            ParamKind::Array(Box::new(ParamKind::Tuple(vec![
                // RemoteTransaction[]
                Box::new(ParamKind::Uint(256)), // timestamp
                Box::new(ParamKind::Address),   // remoteContract
                Box::new(ParamKind::Address),   // sourceMsgSender
                Box::new(ParamKind::Address),   // sourceBeneficiary
                Box::new(ParamKind::Address),   // token
                Box::new(ParamKind::Uint(256)), // amount
                Box::new(ParamKind::Bytes),     // method
                Box::new(ParamKind::Uint(256)), // gas
            ]))),
        ],
        ChainUtils::hex_to_bytes(data)?.as_slice(),
    )
    .unwrap();
    log::info!("decoded {:?}, - {}", dec, dec.as_slice().len());
    let dec: ChainRequestResult<Vec<Token>> = match dec.as_slice() {
        [tuple, txs] => Ok(vec![tuple.clone(), txs.clone()]),
        _ => Err(
            b"Unexpected output. Could not decode local block at first level"
                .as_slice()
                .into(),
        ),
    };
    let dec = dec?;
    log::info!("decoded = 2 | {:?}, - {}", dec, dec.as_slice().len());
    match dec.as_slice() {
        [mined_block, remote_transactions] => {
            let mined_block = mined_block.clone();
            let remote_transactions = remote_transactions.clone();
            log::info!("PRE = Mined block is opened up");
            let block = block_tuple_decoder(mined_block)?;
            log::info!("Mined block is opened up == {:?}", remote_transactions);
            let remote_transactions = remote_transactions
                .to_array()
                .unwrap()
                .into_iter()
                .map(|t| {
                    decode_remote_transaction_from_tuple(t.to_tuple().unwrap().as_slice()).unwrap()
                })
                .collect();
            Ok((block, remote_transactions))
        }
        _ => Err(b"Unexpected output. Could not decode local block"
            .as_slice()
            .into()),
    }
}

fn decode_remote_transaction_from_tuple(dec: &[Token]) -> ChainRequestResult<QpTransaction> {
    match dec {
        [timestamp, remote_contract, source_msg_sender, source_beneficiary, token, amount, method, gas] =>
        {
            let timestamp = timestamp.clone().to_uint().unwrap().as_u64();
            let remote_contract = remote_contract.clone().to_address().unwrap();
            let source_msg_sender = source_msg_sender.clone().to_address().unwrap();
            let source_beneficiary = source_beneficiary.clone().to_address().unwrap();
            let token = token.clone().to_address().unwrap();
            let amount = amount.clone().to_uint().unwrap();
            let method = method.clone().to_bytes().unwrap();
            let gas = gas.clone().to_uint().unwrap().as_u64();
            Ok(QpTransaction {
                timestamp,
                remote_contract,
                source_msg_sender,
                source_beneficiary,
                token,
                amount,
                method,
                gas,
            })
        }
        _ => Err(b"Unexpected output. Could not decode remote transaction"
            .as_slice()
            .into()),
    }
}

impl<T: Config> QuantumPortalClient<T> {
    pub fn new(
        contract: ContractClient,
        signer: ContractClientSignature,
        now: u64,
        block_number: u64,
        eip_712_config: EIP712Config,
    ) -> Self {
        QuantumPortalClient {
            contract,
            signer,
            now,
            block_number,
            eip_712_config,
            _phantom: Default::default(),
        }
    }

    pub fn is_local_block_ready(&self, chain_id: u64) -> ChainRequestResult<bool> {
        let signature = b"isLocalBlockReady(uint64)";
        let res: Box<CallResponse> = self
            .contract
            .call(signature, &[Token::Uint(U256::from(chain_id))])?;
        let val = ChainUtils::hex_to_u256(&res.result)?;
        Ok(!val.is_zero())
    }

    pub fn last_remote_mined_block(&self, chain_id: u64) -> ChainRequestResult<QpLocalBlock> {
        let signature = b"lastRemoteMinedBlock(uint64)";
        let res: Box<CallResponse> = self
            .contract
            .call(signature, &[Token::Uint(U256::from(chain_id))])?;
        self.decode_local_block(res.result.as_slice())
    }

    pub fn last_finalized_block(&self, chain_id: u64) -> ChainRequestResult<QpLocalBlock> {
        let signature = b"lastFinalizedBlock(uint256)";
        let res: Box<CallResponse> = self
            .contract
            .call(signature, &[Token::Uint(U256::from(chain_id))])?;
        self.decode_local_block(res.result.as_slice())
    }

    pub fn last_local_block(&self, chain_id: u64) -> ChainRequestResult<QpLocalBlock> {
        let signature = b"lastLocalBlock(uint256)";
        let res: Box<CallResponse> = self
            .contract
            .call(signature, &[Token::Uint(U256::from(chain_id))])?;
        self.decode_local_block(res.result.as_slice())
    }

    pub fn local_block_by_nonce(
        &self,
        chain_id: u64,
        last_block_nonce: u64,
    ) -> ChainRequestResult<(QpLocalBlock, Vec<QpTransaction>)> {
        let signature = b"localBlockByNonce(uint64,uint64)";
        let res: Box<CallResponse> = self.contract.call(
            signature,
            &[
                Token::Uint(U256::from(chain_id)),
                Token::Uint(U256::from(last_block_nonce)),
            ],
        )?;
        decode_remote_block_and_txs(res.result.as_slice(), local_block_tuple(), |block| {
            log::info!("1-DECODING BLOCK {:?}", block);
            let block = block.to_tuple();
            let block = block.unwrap();
            log::info!("2-DECODING BLOCK {:?}", block);
            Self::decode_local_block_from_tuple(block.as_slice())
        })
    }

    pub fn mined_block_by_nonce(
        &self,
        chain_id: u64,
        last_block_nonce: u64,
    ) -> ChainRequestResult<(QpRemoteBlock, Vec<QpTransaction>)> {
        let signature = b"minedBlockByNonce(uint64,uint64)";
        let res: Box<CallResponse> = self.contract.call(
            signature,
            &[
                Token::Uint(U256::from(chain_id)),
                Token::Uint(U256::from(last_block_nonce)),
            ],
        )?;
        let mined_block_tuple = ParamKind::Tuple(vec![
            // MinedBlock
            Box::new(ParamKind::FixedBytes(32)), // blockHash
            Box::new(ParamKind::Address),        // miner
            Box::new(ParamKind::Uint(256)),      // stake
            Box::new(ParamKind::Uint(256)),      // totalValue
            Box::new(local_block_tuple()),
        ]);
        // let mined_block_tuple = vec![             // MinedBlock
        //                                    ParamKind::FixedBytes(32),    // blockHash
        //                                    ParamKind::Address,           // miner
        //                                    ParamKind::Uint(256),         // stake
        //                                    ParamKind::Uint(256),         // totalValue
        //                                    local_block_tuple()
        // ];
        decode_remote_block_and_txs(res.result.as_slice(), mined_block_tuple, |block| {
            log::info!("Decoding local block, {:?}", block);
            Self::decode_mined_block_from_tuple(block.to_tuple().unwrap().as_slice())
        })
    }

    pub fn create_finalize_transaction(
        &self,
        remote_chain_id: u64,
        block_nonce: u64,
        _finalizer_hash: H256,
        _finalizers: &[Vec<u8>],
    ) -> ChainRequestResult<H256> {
        // because of sp_std, so here are the alternatives:
        // - Manually construct the function call as [u8].
        // function finalize(
        // 	uint256 remoteChainId,
        // 	uint256 blockNonce,
        // 	bytes32 finalizersHash,
        // 	address[] memory finalizers
        // ) ...
        // The last item is a bit complicated, but for now we pass an empty array.
        // Support buytes and dynamic arrays in future
        let finalizer_list: Vec<Token> = vec![];

        let (block_details, _) = self.mined_block_by_nonce(remote_chain_id, block_nonce)?;

        let method_signature =
            b"finalizeSingleSigner(uint256,uint256,bytes32,address[],bytes32,uint64,bytes)";

        // generate randomness for salt
        // let (random_hash, _) = T::PalletRandomness::random_seed();

        // let random_hash = ChainUtils::keccack(b"test1");
        // log::info!("random_hash {:?}", random_hash);

        let salt = Token::FixedBytes(block_details.block_hash.as_ref().to_vec());
        let finalizer_hash = Token::FixedBytes(block_details.block_hash.as_ref().to_vec());

        let current_timestamp = block_details.block_metadata.timestamp;
        // expirt 1hr from now
        let expiry_buffer = core::time::Duration::from_secs(3600u64);
        let expiry_time = current_timestamp.saturating_add(expiry_buffer.as_secs());
        let expiry = Token::Uint(U256::from(expiry_time));

        let multi_sig = self.generate_multi_signature(
            remote_chain_id,
            block_nonce,
            finalizer_hash.clone(),
            finalizer_list.clone(),
            salt.clone(),
            expiry.clone(),
        )?;

        log::info!(
            "Encoded Multisig generated : {:?}",
            sp_std::str::from_utf8(ChainUtils::bytes_to_hex(multi_sig.as_slice()).as_slice())
                .unwrap()
        );

        let inputs = [
            Token::Uint(U256::from(remote_chain_id)),
            Token::Uint(U256::from(block_nonce)),
            finalizer_hash,
            Token::Array(finalizer_list),
            salt,
            expiry,
            Token::Bytes(multi_sig),
        ];

        let res = self.contract.send(
            method_signature,
            &inputs,
            None, //Some(U256::from(1000000 as u64)), // None,
            None, // Some(U256::from(10000000000 as u64)), // None,
            U256::zero(),
            None,
            self.signer.from,
            &self.signer,
        )?;
        Ok(res)
    }

    /// Returns the multiSignature to sign finalize transactions
    /// The function will
    /// 1. Generate the domain seperator values, encoded and hashed
    /// 2. Generate the message hash from the args of the finalize call and encoded it to the signature
    /// 3. Generate the eip_712 type hash for the ValidateAuthoritySignature function
    pub fn generate_multi_signature(
        &self,
        remote_chain_id: u64,
        block_nonce: u64,
        finalizer_hash: Token,
        finalizer_list: Vec<Token>,
        salt: Token,
        expiry: Token,
    ) -> Result<Vec<u8>, TransactionCreationError> {
        // Generate the domain seperator hash, the hash is generated from the given arguments
        let domain_seperator_hash = EIP712Utils::generate_eip_712_domain_seperator_hash(
            &self.eip_712_config.contract_name,     // ContractName
            &self.eip_712_config.contract_version,  // ContractVersion
            self.contract.chain_id,                 // ChainId
            &self.eip_712_config.verifying_address, // VerifyingAddress
        );
        log::info!("domain_seperator_hash {:?}", domain_seperator_hash);

        // Generate the finalize method sigature to encode the finalize call
        let finalize_method_signature = b"Finalize(uint256 remoteChainId,uint256 blockNonce,bytes32 finalizersHash,address[] finalizers,bytes32 salt,uint64 expiry)";
        let finalize_method_signature_hash = ChainUtils::keccack(finalize_method_signature);
        log::info!(
            "finalize_method_signature_hash {:?}",
            finalize_method_signature_hash
        );

        log::info!("remote_chain_id {:?}", remote_chain_id);
        log::info!("block_nonde {:?}", block_nonce);
        log::info!("finalizer_hash {:?}", finalizer_hash);
        log::info!("finalizer_list {:?}", finalizer_list);
        log::info!("salt {:?}", salt);
        log::info!("expiry {:?}", expiry);

        // encode the finalize call to the expected format
        let encoded_message_hash = EIP712Utils::get_encoded_hash(vec![
            Token::FixedBytes(Vec::from(finalize_method_signature_hash.as_bytes())), // finalize method signature hash
            Token::Uint(U256::from(remote_chain_id)), // remote chain id
            Token::Uint(U256::from(block_nonce)),     // block nonce
            finalizer_hash,                           // finalizers hash
            Token::Array(finalizer_list),             // finalizers
            salt.clone(),                             // salt
            expiry.clone(),                           // expiry
        ]);
        log::info!("encoded_message_hash {:?}", encoded_message_hash);

        // Generate the ValidateAuthoritySignature method signature to encode the eip_args
        let method_signature = b"ValidateAuthoritySignature(uint256 action,bytes32 msgHash,bytes32 salt,uint64 expiry)";
        let method_hash = ChainUtils::keccack(method_signature);
        log::info!("method_hash {:?}", method_hash);

        // Generate the encoded eip message
        let eip_args_hash = EIP712Utils::get_encoded_hash(vec![
            Token::FixedBytes(Vec::from(method_hash.as_bytes())), // method hash
            Token::Uint(U256::from(1)),                           // action
            Token::FixedBytes(Vec::from(encoded_message_hash.as_bytes())), // msgHash
            salt,                                                 // salt
            expiry,                                               // expiry
        ]);
        log::info!("eip_args_hash {:?}", eip_args_hash);

        let eip_712_hash =
            EIP712Utils::generate_eip_712_hash(&domain_seperator_hash[..], &eip_args_hash[..]);
        log::info!("EIP712 Hash {:?}", eip_712_hash);

        // Sign the eip message, we only consider a single signer here since we only expect a single key in the keystore
        // TODO : Add the ability for multiple signers
        let multi_sig_bytes = self.signer.signer(&eip_712_hash)?;

        // Compute multisig format
        // This computation makes it match the implementation we have in qp smart contracts repo
        // refer https://github.com/ferrumnet/quantum-portal-smart-contracts/blob/326341cdfcb55052437393228f1d58e014c90f7b/test/common/Eip712Utils.ts#L93
        let mut multisig_compressed: Vec<u8> = multi_sig_bytes.0[0..64].to_vec();
        multisig_compressed.extend([28u8]);
        multisig_compressed.extend([0u8; 31]);

        log::info!(
            "Extended signature of size {}: {}",
            multisig_compressed.len(),
            sp_std::str::from_utf8(
                ChainUtils::bytes_to_hex(multisig_compressed.as_slice()).as_slice()
            )
            .unwrap()
        );

        Ok(multisig_compressed)
    }

    pub fn create_mine_transaction(
        &self,
        remote_chain_id: u64,
        block_nonce: u64,
        txs: &Vec<QpTransaction>,
    ) -> ChainRequestResult<H256> {
        let method_signature = b"mineRemoteBlock(uint64,uint64,(uint64,address,address,address,address,uint256,bytes,uint256)[],bytes32,uint64,bytes)";

        let salt = Token::FixedBytes(vec![
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1,
        ]);
        let expiry = Token::Uint(U256::from(2147483647));
        let multi_sig = Token::Bytes(vec![
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 1, 1,
        ]);

        let tx_vec = txs
            .iter()
            .map(|t| {
                Token::Tuple(vec![
                    Token::Uint(U256::from(t.timestamp)),
                    Token::Address(t.remote_contract),
                    Token::Address(t.source_msg_sender),
                    Token::Address(t.source_beneficiary),
                    Token::Address(t.token),
                    Token::Uint(t.amount),
                    Token::Bytes(t.method.clone()),
                    Token::Uint(U256::from(t.gas)),
                ])
            })
            .collect();

        let res = self.contract.send(
            method_signature,
            &[
                Token::Uint(U256::from(remote_chain_id)),
                Token::Uint(U256::from(block_nonce)),
                Token::Array(tx_vec),
                salt,
                expiry,
                multi_sig,
            ],
            None, // Some(U256::from(1000000 as u32)), // None,
            None, // Some(U256::from(60000000000 as u64)), // None,
            U256::zero(),
            None,
            self.signer.from,
            &self.signer,
        )?;
        Ok(res)
    }

    pub fn finalize(&self, chain_id: u64) -> ChainRequestResult<Option<H256>> {
        log::info!("finalize({})", chain_id);
        let block = self.last_remote_mined_block(chain_id)?;
        log::info!("finalize-last_remote_mined_block({:?})", &block);
        let last_fin = self.last_finalized_block(chain_id)?;
        log::info!("finalize-last_finalized_block({:?})", &last_fin);
        if block.nonce > last_fin.nonce {
            log::info!("Calling mgr.finalize({}, {})", chain_id, block.nonce);
            Ok(Some(self.create_finalize_transaction(
                chain_id,
                block.nonce,
                H256::zero(),
                &[self.signer.get_signer_address()],
            )?))
        } else {
            log::info!("Nothing to finalize for ({})", chain_id);
            Ok(None)
        }
    }

    pub fn mine(&self, remote_client: &QuantumPortalClient<T>) -> ChainRequestResult<Option<H256>> {
        let local_chain = self.contract.chain_id;
        let remote_chain = remote_client.contract.chain_id;
        log::info!("mine({} => {})", remote_chain, local_chain);
        let block_ready = remote_client.is_local_block_ready(local_chain)?;
        log::info!("local block ready? {}", block_ready);
        if !block_ready {
            return Ok(None);
        }
        log::info!("Getting last local block");
        let last_block = remote_client.last_local_block(local_chain)?;
        log::info!("Last local block is {:?}", last_block);
        let last_mined_block = self.last_remote_mined_block(remote_chain)?;
        log::info!("Local block f remote (chain {}) nonce is {}. Remote mined block on local (chain {}) is {}",
			remote_chain, last_block.nonce, local_chain, last_mined_block.nonce);
        if last_mined_block.nonce >= last_block.nonce {
            log::info!("Nothing to mine!");
            return Ok(None);
        }
        log::info!(
            "Last block is on chain1 for target {} is {}",
            local_chain,
            last_block.nonce
        );
        let mined_block = self.mined_block_by_nonce(remote_chain, last_block.nonce)?;
        let already_mined = !mined_block.0.block_hash.eq(&ZERO_HASH);
        if already_mined {
            return Err(ChainRequestError::RemoteBlockAlreadyMined);
        }
        log::info!("Getting source block?");
        let source_block = remote_client.local_block_by_nonce(local_chain, last_block.nonce)?;
        let default_qp_transaction = QpTransaction::default();
        log::info!(
            "Source block is GOT\n{:?}\n{:?}",
            source_block.0,
            if !source_block.1.is_empty() {
                source_block.1.get(0).unwrap()
            } else {
                &default_qp_transaction
            }
        );
        let txs = source_block.1;
        log::info!(
            "About to mine block {}:{}",
            remote_chain,
            source_block.0.nonce
        );
        Ok(Some(self.create_mine_transaction(
            remote_chain,
            source_block.0.nonce,
            &txs,
        )?))
    }

    fn decode_local_block(&self, data: &[u8]) -> ChainRequestResult<QpLocalBlock> {
        let dec = decode(
            // &[local_block_tuple()],
            local_block_tuple0().as_slice(),
            ChainUtils::hex_to_bytes(data)?.as_slice(),
        )
        .unwrap();
        Self::decode_local_block_from_tuple(dec.as_slice())
    }

    fn decode_local_block_from_tuple(dec: &[Token]) -> ChainRequestResult<QpLocalBlock> {
        log::info!("Decoding local block, {:?}", dec);
        match dec {
            [chain_id, nonce, timestamp] => {
                let chain_id = chain_id.clone().to_uint();
                let nonce = nonce.clone().to_uint();
                let timestamp = timestamp.clone().to_uint();
                Ok(QpLocalBlock {
                    chain_id: chain_id.unwrap().as_u64(),
                    nonce: nonce.unwrap().as_u64(),
                    timestamp: timestamp.unwrap().as_u64(),
                })
            }
            _ => Err(b"Unexpected output. Could not decode local block"
                .as_slice()
                .into()),
        }
    }

    fn decode_mined_block_from_tuple(dec: &[Token]) -> ChainRequestResult<QpRemoteBlock> {
        log::info!("decode_mined_block_from_tuple {:?}", dec);
        match dec {
            [block_hash, miner, stake, total_value, block_metadata] => {
                log::info!(
                    "D {:?}::{:?}:{:?}:{:?}::{:?}",
                    block_hash,
                    miner,
                    stake,
                    total_value,
                    block_metadata
                );
                let block_hash = block_hash.clone();
                let miner = miner.clone();
                let stake = stake.clone();
                let total_value = total_value.clone();
                let block_metadata = block_metadata.clone();
                log::info!("Decoding block metadata");
                let block_metadata =
                    Self::decode_local_block_from_tuple(&block_metadata.to_tuple().unwrap())?;
                log::info!("DecodED block metadata");
                Ok(QpRemoteBlock {
                    block_hash: H256::from_slice(block_hash.to_fixed_bytes().unwrap().as_slice()),
                    miner: miner.to_address().unwrap(),
                    stake: stake.to_uint().unwrap(),
                    total_value: total_value.to_uint().unwrap(),
                    block_metadata,
                })
            }
            _ => Err(b"Unexpected output. Could not decode mined block"
                .as_slice()
                .into()),
        }
    }
}
