// This file is part of Substrate.

// Copyright (C) 2017-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use clap::Parser;
// Substrate
use sc_cli::{ChainSpec, RuntimeVersion, SubstrateCli};
use sc_service::{DatabaseSource, PartialComponents};
// Frontier
use fc_db::frontier_database_dir;

use crate::{
    chain_spec,
    cli::{Cli, Subcommand},
    service::{self, db_config_dir},
};

impl SubstrateCli for Cli {
    fn impl_name() -> String {
        "FerrumX Node".into()
    }

    fn impl_version() -> String {
        env!("SUBSTRATE_CLI_IMPL_VERSION").into()
    }

    fn description() -> String {
        env!("CARGO_PKG_DESCRIPTION").into()
    }

    fn author() -> String {
        env!("CARGO_PKG_AUTHORS").into()
    }

    fn support_url() -> String {
        "support.anonymous.an".into()
    }

    fn copyright_start_year() -> i32 {
        2022
    }

    fn load_spec(&self, id: &str) -> Result<Box<dyn sc_service::ChainSpec>, String> {
        Ok(match id {
            "dev" => Box::new(chain_spec::development_config(self)?),
            "alpha-testnet" => Box::new(chain_spec::alpha_testnet_config(self)?),
            "" | "local" => Box::new(chain_spec::local_testnet_config(self)?),
            path => Box::new(chain_spec::ChainSpec::from_json_file(
                std::path::PathBuf::from(path),
            )?),
        })
    }

    fn native_runtime_version(_: &Box<dyn ChainSpec>) -> &'static RuntimeVersion {
        &ferrum_x_runtime::VERSION
    }
}

/// Parse and run command line arguments
pub fn run() -> sc_cli::Result<()> {
    let cli = Cli::parse();

    match &cli.subcommand {
        Some(Subcommand::Key(cmd)) => cmd.run(&cli),
        Some(Subcommand::BuildSpec(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| cmd.run(config.chain_spec, config.network))
        }
        Some(Subcommand::CheckBlock(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    import_queue,
                    ..
                } = service::new_partial(&config, &cli)?;
                Ok((cmd.run(client, import_queue), task_manager))
            })
        }
        Some(Subcommand::ExportBlocks(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    ..
                } = service::new_partial(&config, &cli)?;
                Ok((cmd.run(client, config.database), task_manager))
            })
        }
        Some(Subcommand::ExportState(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    ..
                } = service::new_partial(&config, &cli)?;
                Ok((cmd.run(client, config.chain_spec), task_manager))
            })
        }
        Some(Subcommand::ImportBlocks(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    import_queue,
                    ..
                } = service::new_partial(&config, &cli)?;
                Ok((cmd.run(client, import_queue), task_manager))
            })
        }
        Some(Subcommand::PurgeChain(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| {
                // Remove Frontier offchain db
                let db_config_dir = db_config_dir(&config);
                let frontier_database_config = match config.database {
                    DatabaseSource::RocksDb { .. } => DatabaseSource::RocksDb {
                        path: frontier_database_dir(&db_config_dir, "db"),
                        cache_size: 0,
                    },
                    DatabaseSource::ParityDb { .. } => DatabaseSource::ParityDb {
                        path: frontier_database_dir(&db_config_dir, "paritydb"),
                    },
                    _ => {
                        return Err(format!("Cannot purge `{:?}` database", config.database).into())
                    }
                };
                cmd.run(frontier_database_config)?;
                cmd.run(config.database)
            })
        }
        Some(Subcommand::Revert(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents {
                    client,
                    task_manager,
                    backend,
                    ..
                } = service::new_partial(&config, &cli)?;
                let aux_revert = Box::new(move |client, _, blocks| {
                    sc_finality_grandpa::revert(client, blocks)?;
                    Ok(())
                });
                Ok((cmd.run(client, backend, Some(aux_revert)), task_manager))
            })
        }
        #[cfg(feature = "runtime-benchmarks")]
        Some(Subcommand::Benchmark(cmd)) => {
            use crate::benchmarking::{
                inherent_benchmark_data, RemarkBuilder, TransferKeepAliveBuilder,
            };
            use ferrum_x_runtime::{Block, ExistentialDeposit};
            use frame_benchmarking_cli::{
                BenchmarkCmd, ExtrinsicFactory, SUBSTRATE_REFERENCE_HARDWARE,
            };

            let runner = cli.create_runner(cmd)?;
            match cmd {
                BenchmarkCmd::Pallet(cmd) => {
                    runner.sync_run(|config| cmd.run::<Block, service::ExecutorDispatch>(config))
                }
                BenchmarkCmd::Block(cmd) => runner.sync_run(|config| {
                    let PartialComponents { client, .. } = service::new_partial(&config, &cli)?;
                    cmd.run(client)
                }),
                BenchmarkCmd::Storage(cmd) => runner.sync_run(|config| {
                    let PartialComponents {
                        client, backend, ..
                    } = service::new_partial(&config, &cli)?;
                    let db = backend.expose_db();
                    let storage = backend.expose_storage();

                    cmd.run(config, client, db, storage)
                }),
                BenchmarkCmd::Overhead(cmd) => runner.sync_run(|config| {
                    let PartialComponents { client, .. } = service::new_partial(&config, &cli)?;
                    let ext_builder = RemarkBuilder::new(client.clone());

                    cmd.run(
                        config,
                        client,
                        inherent_benchmark_data()?,
                        Vec::new(),
                        &ext_builder,
                    )
                }),
                BenchmarkCmd::Extrinsic(cmd) => runner.sync_run(|config| {
                    let PartialComponents { client, .. } = service::new_partial(&config, &cli)?;
                    // Register the *Remark* and *TKA* builders.
                    let ext_factory = ExtrinsicFactory(vec![
                        Box::new(RemarkBuilder::new(client.clone())),
                        Box::new(TransferKeepAliveBuilder::new(
                            client.clone(),
                            sp_keyring::Sr25519Keyring::Alice.to_account_id(),
                            ExistentialDeposit::get(),
                        )),
                    ]);

                    cmd.run(client, inherent_benchmark_data()?, Vec::new(), &ext_factory)
                }),
                BenchmarkCmd::Machine(cmd) => {
                    runner.sync_run(|config| cmd.run(&config, SUBSTRATE_REFERENCE_HARDWARE.clone()))
                }
            }
        }
        #[cfg(not(feature = "runtime-benchmarks"))]
        Some(Subcommand::Benchmark) => Err("Benchmarking wasn't enabled when building the node. \
			You can enable it with `--features runtime-benchmarks`."
            .into()),
        Some(Subcommand::FrontierDb(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| {
                let PartialComponents { client, other, .. } = service::new_partial(&config, &cli)?;
                let frontier_backend = other.2;
                cmd.run::<_, ferrum_x_runtime::opaque::Block>(client, frontier_backend)
            })
        }
        None => {
            let runner = cli.create_runner(&cli.run.base)?;
            runner.run_node_until_exit(|config| async move {
                service::new_full(config, &cli).map_err(sc_cli::Error::Service)
            })
        }
    }
}
