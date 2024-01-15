use balance::borrow::{Borrow, BorrowId};
use balance::deposit::{Deposit, DepositId};
use balance::reserve::Reserve;
use balance::token_receiver::OpenPositionRequest;
pub use balance::BalancesMap;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::UnorderedMap;
use near_sdk::{env, ext_contract, near_bindgen};
use near_sdk::{AccountId, PanicOnDefault};
use nft::metadata::{NFTContractMetadata, Token, TokenId, TokenMetadata};
use pool::Pool;

pub use crate::balance::*;
use crate::errors::*;
use crate::nft::nft_core::NonFungibleTokenCore;
use crate::position::Position;

pub mod balance;
mod errors;
pub mod pool;
pub mod position;
mod token_receiver;

use near_sdk::collections::{LazyOption, LookupMap, UnorderedSet};
use near_sdk::json_types::{Base64VecU8, U128};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{Balance, CryptoHash, Promise, PromiseOrValue};
use std::collections::HashMap;

mod action;
mod borrow;
mod deposit;
mod nft;
mod reserve;

#[derive(BorshSerialize)]
pub enum StorageKey {
    TokensPerOwner,
    TokenPerOwnerInner { account_id_hash: CryptoHash },
    TokensById,
    TokenMetadataById,
    NFTContractMetadata,
    Balances,
    Reserves,
    Borrows,
}

pub const NFT_METADATA_SPEC: &str = "1.0.0";
pub const NFT_STANDARD_NAME: &str = "nep171";
pub const BASIS_POINT: f64 = 1.0001;
pub const BASIS_POINT_TO_PERCENT: f64 = 10000.0;
pub const APR_DEPOSIT: u16 = 500;
pub const APR_BORROW: u16 = 1000;

/// Maximum Loan-to-Value (LTV) ratio
/// This is the maximum ratio of the loan amount to the value of the collateral.
/// For example, if LTV_MAX is 0.8, you can borrow up to 80% of the value of your collateral.
pub const LTV_MAX: f64 = 0.8;

pub const TGAS: u64 = 1000000000000;

type Pair = (AccountId, AccountId);

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub owner_id: AccountId,
    pub pools: Vec<Pool>,
    //  Accounts registered, keeping track all the amounts deposited
    pub balances_map: BalancesMap,
    pub tokens_per_owner: LookupMap<AccountId, UnorderedSet<TokenId>>,
    pub tokens_by_id: LookupMap<TokenId, Token>,
    pub token_metadata_by_id: UnorderedMap<TokenId, TokenMetadata>,
    pub metadata: LazyOption<NFTContractMetadata>,
    pub positions_opened: u128,
    pub deposits: HashMap<AccountId, HashMap<TokenId, Deposit>>,
    pub deposits_created_number: DepositId,
    pub reserves: UnorderedMap<AccountId, Reserve>,
    pub borrows: UnorderedMap<BorrowId, Borrow>,
    pub borrows_number: BorrowId,
    pub routes: HashMap<Pair, Vec<i32>>,
    pub routes_counter: i32,
    pub open_position_requests: HashMap<usize, OpenPositionRequest>,
}

#[ext_contract(ext_self)]
pub trait SelfCallbacks {
    fn nft_transfer(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
    );
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(owner_id: AccountId) -> Self {
        let metadata = NFTContractMetadata {
            spec: "nft-1.0.0".to_string(),
            name: "Crisp Exchange Contract".to_string(),
            symbol: "CRISP.EX".to_string(),
            icon: None,
            base_uri: None,
            reference: None,
            reference_hash: None,
        };
        Self {
            owner_id,
            pools: Vec::new(),
            balances_map: UnorderedMap::new(StorageKey::Balances.try_to_vec().unwrap()),
            tokens_per_owner: LookupMap::new(StorageKey::TokensPerOwner.try_to_vec().unwrap()),
            tokens_by_id: LookupMap::new(StorageKey::TokensById.try_to_vec().unwrap()),
            token_metadata_by_id: UnorderedMap::new(
                StorageKey::TokenMetadataById.try_to_vec().unwrap(),
            ),
            metadata: LazyOption::new(
                StorageKey::NFTContractMetadata.try_to_vec().unwrap(),
                Some(&metadata),
            ),
            positions_opened: 0,
            deposits: HashMap::new(),
            deposits_created_number: 0,
            reserves: UnorderedMap::new(StorageKey::Reserves.try_to_vec().unwrap()),
            borrows: UnorderedMap::new(StorageKey::Borrows.try_to_vec().unwrap()),
            borrows_number: 0,
            routes: HashMap::new(),
            routes_counter: 1,
            open_position_requests: HashMap::new(),
        }
    }

    #[private]
    pub fn create_pool(
        &mut self,
        token1: AccountId,
        token2: AccountId,
        initial_price: f64,
        protocol_fee: u16,
        rewards: u16,
    ) -> i32 {
        self.pools.push(Pool::new(
            token1.clone(),
            token2.clone(),
            initial_price,
            protocol_fee,
            rewards,
        ));
        assert!(!self.routes.contains_key(&(token1.clone(), token2.clone())));
        assert!(!self.routes.contains_key(&(token2.clone(), token1.clone())));
        let routes = self.routes.clone();
        for (pair, route) in routes {
            if pair.0 == token1 {
                let mut new_route = route.clone();
                new_route.push(self.routes_counter);
                self.routes
                    .insert((pair.0.clone(), token2.clone()), new_route.clone());
                self.routes.insert(
                    (token2.clone(), pair.0.clone()),
                    Self::modify_vec(new_route),
                );
            } else if pair.0 == token2 {
                let mut new_route = route.clone();
                new_route.push(self.routes_counter);
                self.routes
                    .insert((pair.0.clone(), token1.clone()), new_route.clone());
                self.routes.insert(
                    (token1.clone(), pair.0.clone()),
                    Self::modify_vec(new_route),
                );
            } else if pair.1 == token1 {
                let mut new_route = route.clone();
                new_route.push(self.routes_counter);
                self.routes
                    .insert((pair.0.clone(), token2.clone()), new_route.clone());
                self.routes.insert(
                    (token2.clone(), pair.0.clone()),
                    Self::modify_vec(new_route),
                );
            } else if pair.1 == token2 {
                let mut new_route = route.clone();
                new_route.push(-self.routes_counter);
                self.routes
                    .insert((pair.0.clone(), token1.clone()), new_route.clone());
                self.routes.insert(
                    (token1.clone(), pair.0.clone()),
                    Self::modify_vec(new_route),
                );
            }
        }
        self.routes
            .insert((token1.clone(), token2.clone()), vec![self.routes_counter]);
        self.routes
            .insert((token2.clone(), token1.clone()), vec![-self.routes_counter]);
        self.routes_counter += 1;
        self.routes_counter - 1
    }

    fn modify_vec(mut vec: Vec<i32>) -> Vec<i32> {
        vec.reverse();
        vec.into_iter().map(|x| -x).collect()
    }

    pub fn positions_opened(&self) -> u128 {
        self.positions_opened
    }

    #[private]
    pub fn remove_pool(&mut self, pool_id: usize) {
        self.assert_pool_exists(pool_id);
        self.pools.remove(pool_id);
    }

    pub fn get_pools(&self) -> Vec<Pool> {
        self.pools.clone()
    }

    fn assert_pool_exists(&self, pool_id: usize) {
        assert!(pool_id < self.pools.len(), "{}", BAD_POOL_ID);
    }

    fn assert_account_owns_nft(account_id: &AccountId, nft_owner: &AccountId) {
        assert!(account_id == nft_owner);
    }

    pub fn get_pool(&self, pool_id: usize) -> Pool {
        self.assert_pool_exists(pool_id);
        self.pools[pool_id].clone()
    }

    pub fn get_balance(&self, account_id: &AccountId, token: &AccountId) -> U128 {
        let balance = match self.balances_map.get(account_id) {
            None => panic!("{}", BAL1),
            Some(balance) => balance.get(token),
        };
        if let Some(amount) = balance {
            amount.into()
        } else {
            U128(0)
        }
    }

    pub fn get_balance_all_tokens(&self, account_id: &AccountId) -> String {
        if let Some(balance) = self.balances_map.get(account_id) {
            let mut result = String::new();
            for (token, amount) in balance.iter() {
                result += &format!("{token}: {amount}, ");
            }
            result
        } else {
            String::new()
        }
    }

    pub fn withdraw(&mut self, token: &AccountId, amount: U128) {
        let account_id = env::signer_account_id();
        let amount: u128 = amount.into();
        self.balance_withdraw(&account_id, token, amount);
    }

    pub fn get_return(&self, pool_id: usize, token_in: &AccountId, amount_in: U128) -> U128 {
        let pool = self.get_pool(pool_id);
        let swap_result =
            pool.get_swap_result(token_in, amount_in.into(), pool::SwapDirection::Return);
        let fees_amount = swap_result.amount * (pool.protocol_fee as f64 + pool.rewards as f64)
            / BASIS_POINT_TO_PERCENT;
        (swap_result.amount.round() as u128 - fees_amount.round() as u128).into()
    }

    // not too accurate because of fees :( [TO DO]
    pub fn get_expense(&self, pool_id: usize, token_out: &AccountId, amount_out: U128) -> U128 {
        let pool = self.get_pool(pool_id);
        let swap_result =
            pool.get_swap_result(token_out, amount_out.into(), pool::SwapDirection::Expense);
        (swap_result.amount.round() as u128).into()
    }

    pub fn get_price(&self, pool_id: usize) -> f64 {
        self.get_pool(pool_id).sqrt_price.powf(2.0)
    }

    pub fn swap(
        &mut self,
        pool_id: usize,
        token_in: &AccountId,
        amount_in: U128,
        token_out: &AccountId,
    ) -> U128 {
        self.assert_pool_exists(pool_id);
        let account_id = env::signer_account_id();
        let amount_in: u128 = amount_in.into();
        self.decrease_balance(&account_id, token_in, amount_in);
        let pool = &mut self.pools[pool_id];
        let swap_result = pool.get_swap_result(token_in, amount_in, pool::SwapDirection::Return);
        let fees_amount = swap_result.amount * (pool.protocol_fee as f64 + pool.rewards as f64)
            / BASIS_POINT_TO_PERCENT;
        self.apply_collected_fees(&swap_result.collected_fees, token_out);
        let result_amount = swap_result.amount.round() as u128 - fees_amount.round() as u128;
        self.increase_balance(&account_id, token_out, result_amount);
        let pool = &mut self.pools[pool_id];
        pool.apply_swap_result(&swap_result);
        pool.refresh(env::block_timestamp());
        result_amount.into()
    }

    pub fn swap_multihope(
        &mut self,
        token_in: &AccountId,
        amount_in: U128,
        token_out: &AccountId,
    ) -> U128 {
        let mut amount = amount_in;
        let route = self
            .routes
            .get(&(token_in.to_string(), token_out.to_string()))
            .expect(SWP0)
            .clone();
        for pool_id in route {
            if pool_id > 0 {
                let pool = self.pools[pool_id as usize].clone();
                amount = self.swap(pool_id as usize, &pool.token0, amount, &pool.token1);
            } else {
                let pool = self.pools[-pool_id as usize].clone();
                amount = self.swap(-pool_id as usize, &pool.token1, amount, &pool.token0);
            }
        }
        amount
    }

    pub fn open_position(
        &mut self,
        pool_id: usize,
        token0_liquidity: Option<U128>,
        token1_liquidity: Option<U128>,
        lower_bound_price: f64,
        upper_bound_price: f64,
    ) -> u128 {
        self.assert_pool_exists(pool_id);
        let position_id = self.positions_opened;
        self.positions_opened += 1;
        let pool = &self.pools[pool_id];
        let account_id = env::signer_account_id();
        let position = Position::new(
            account_id.clone(),
            token0_liquidity,
            token1_liquidity,
            lower_bound_price,
            upper_bound_price,
            pool.sqrt_price,
        );
        let token0 = pool.token0.clone();
        let token1 = pool.token1.clone();
        self.decrease_balance(&account_id, &token0, position.token0_locked.round() as u128);
        self.decrease_balance(&account_id, &token1, position.token1_locked.round() as u128);
        let pool = &mut self.pools[pool_id];
        pool.open_position(position_id, position.clone());
        pool.refresh(env::block_timestamp());
        let metadata = TokenMetadata::new(pool_id, position_id, &position);
        self.nft_mint(position_id.to_string(), account_id.clone(), metadata);
        position_id
    }

    pub fn close_position(&mut self, pool_id: usize, position_id: u128) {
        self.assert_pool_exists(pool_id);
        let pool = &self.pools[pool_id];
        let account_id = env::predecessor_account_id();
        let token = self.tokens_by_id.get(&position_id.to_string()).expect(NFT0);
        Self::assert_account_owns_nft(&account_id, &token.owner_id);
        let position = pool.positions.get(&position_id).expect(PST0);
        let amount0 = position.token0_locked.round() as u128;
        let amount1 = position.token1_locked.round() as u128;
        let token0 = pool.token0.clone();
        let token1 = pool.token1.clone();
        self.increase_balance(&account_id, &token0, amount0);
        self.increase_balance(&account_id, &token1, amount1);
        let pool = &mut self.pools[pool_id];
        pool.close_position(position_id);
    }

    pub fn add_liquidity(
        &mut self,
        pool_id: usize,
        position_id: u128,
        token0_liquidity: Option<U128>,
        token1_liquidity: Option<U128>,
    ) {
        self.assert_pool_exists(pool_id);
        let pool = &mut self.pools[pool_id];
        let account_id = env::predecessor_account_id();
        let token = self.tokens_by_id.get(&position_id.to_string()).expect(NFT0);
        Self::assert_account_owns_nft(&account_id, &token.owner_id);
        let mut position = pool.positions.get(&position_id).expect(PST0).clone();
        let token0_locked_before = position.token0_locked as u128;
        let token1_locked_before = position.token1_locked as u128;
        position.add_liquidity(token0_liquidity, token1_liquidity, pool.sqrt_price);
        let token0_locked_after = position.token0_locked as u128;
        let token1_locked_after = position.token1_locked as u128;
        pool.positions.insert(position_id, position);
        pool.refresh(env::block_timestamp());
        let token0 = pool.token0.to_string();
        let token1 = pool.token1.to_string();
        self.decrease_balance(
            &account_id,
            &token0,
            token0_locked_after - token0_locked_before,
        );
        self.decrease_balance(
            &account_id,
            &token1,
            token1_locked_after - token1_locked_before,
        );
    }

    pub fn remove_liquidity(
        &mut self,
        pool_id: usize,
        position_id: u128,
        token0_liquidity: Option<U128>,
        token1_liquidity: Option<U128>,
    ) {
        self.assert_pool_exists(pool_id);
        let pool = &mut self.pools[pool_id];
        let account_id = env::predecessor_account_id();
        let token = self.tokens_by_id.get(&position_id.to_string()).expect(NFT0);
        Self::assert_account_owns_nft(&account_id, &token.owner_id);
        let mut position = pool.positions.get(&position_id).expect(PST0).clone();
        let token0_locked_before = position.token0_locked as u128;
        let token1_locked_before = position.token1_locked as u128;
        position.remove_liquidity(token0_liquidity, token1_liquidity, pool.sqrt_price);
        let token0_locked_after = position.token0_locked as u128;
        let token1_locked_after = position.token1_locked as u128;
        pool.positions.insert(position_id, position);
        pool.refresh(env::block_timestamp());
        let token0 = pool.token0.to_string();
        let token1 = pool.token1.to_string();
        self.increase_balance(
            &account_id,
            &token0,
            token0_locked_before - token0_locked_after,
        );
        self.increase_balance(
            &account_id,
            &token1,
            token1_locked_before - token1_locked_after,
        );
    }

    #[private]
    pub fn create_reserve(&mut self, reserve_token: &AccountId) {
        let reserve = Reserve::default();
        self.reserves.insert(reserve_token, &reserve);
    }

    pub fn create_deposit(&mut self, asset: &AccountId, amount: U128) {
        let account_id = env::predecessor_account_id();
        let timestamp = env::block_timestamp();
        if let Some(map) = self.deposits.get(&account_id) {
            if let Some(deposit) = map.get(asset) {
                let old_amount = deposit.amount;
                let old_growth = deposit.growth;
                let deposit = Deposit {
                    owner_id: account_id.clone(),
                    asset: asset.clone(),
                    amount: amount.0 + old_amount,
                    timestamp,
                    last_update_timestamp: timestamp,
                    apr: APR_DEPOSIT,
                    growth: old_growth,
                };
                let mut map = map.clone();
                map.insert(asset.clone(), deposit);
                self.deposits.insert(account_id.clone(), map);
            } else {
                let deposit = Deposit {
                    owner_id: account_id.clone(),
                    asset: asset.clone(),
                    amount: amount.0,
                    timestamp,
                    last_update_timestamp: timestamp,
                    apr: APR_DEPOSIT,
                    growth: 0,
                };
                let mut map = map.clone();
                map.insert(asset.clone(), deposit);
                self.deposits.insert(account_id.clone(), map.clone());
            }
        } else {
            let deposit = Deposit {
                owner_id: account_id.clone(),
                asset: asset.clone(),
                amount: amount.0,
                timestamp,
                last_update_timestamp: timestamp,
                apr: APR_DEPOSIT,
                growth: 0,
            };
            let mut map = HashMap::new();
            map.insert(asset.clone(), deposit);
            self.deposits.insert(account_id.clone(), map);
        }
        self.decrease_balance(&account_id, &asset.to_string(), amount.0);

        let mut reserve = self.reserves.get(&asset).expect(RSR0);
        reserve.increase_deposit(amount.0);
        self.reserves.insert(&asset, &reserve);
    }

    pub fn close_deposit(&mut self, asset: &AccountId, amount: U128) {
        let account_id = env::predecessor_account_id();
        let timestamp = env::block_timestamp();
        let map = self.deposits.get(&account_id).unwrap();
        let deposit = map.get(asset).unwrap();
        let old_amount = deposit.amount;
        assert!(old_amount >= amount.0);
        let old_growth = deposit.growth;
        let deposit = Deposit {
            owner_id: account_id.clone(),
            asset: asset.clone(),
            amount: old_amount - amount.0,
            timestamp,
            last_update_timestamp: timestamp,
            apr: APR_DEPOSIT,
            growth: old_growth,
        };
        let mut map = map.clone();
        map.insert(asset.clone(), deposit);
        self.deposits.insert(account_id.clone(), map.clone());
        self.increase_balance(&account_id, &asset.to_string(), amount.0);

        let mut reserve = self.reserves.get(&asset).expect(RSR0);
        reserve.decrease_deposit(amount.0);
        self.reserves.insert(&asset, &reserve);
    }

    pub fn refresh_deposits_growth(&mut self) {
        let current_timestamp = env::block_timestamp();
        for (_, map) in &mut self.deposits {
            for (_, deposit) in map {
                deposit.refresh_growth(current_timestamp);
            }
        }
    }

    // #[allow(unused_assignments)]
    // pub fn take_deposit_growth(&mut self, asset: AccountId, amount: U128) -> U128 {
    //     let account_id = env::predecessor_account_id();
    //     let mut growth = 0;
    //     let map = self.deposits.get_mut(&account_id).expect(DPS0);
    //     let deposit
    //     assert_eq!(deposit.owner_id, account_id, "{}", DPS1);
    //     deposit.refresh_growth(env::block_timestamp());
    //     growth = deposit.take_growth(amount.0);
    //     asset = Some(deposit.asset.clone());
    //     if let Some(asset) = asset {
    //         self.increase_balance(&account_id, &asset, growth);
    //         return growth.into();
    //     }
    //     0.into()
    // }

    pub fn get_account_deposits(&self, account_id: AccountId) -> HashMap<TokenId, Deposit> {
        if let Some(deposit) = self.deposits.get(&account_id) {
            deposit.clone()
        } else {
            HashMap::new()
        }
    }

    // #[payable]
    // pub fn supply_collateral_and_borrow_simple(
    //     &mut self,
    //     pool_id: usize,
    //     position_id: u128,
    // ) -> U128 {
    //     let account_id = env::predecessor_account_id();
    //     let pool = &self.pools[pool_id];
    //     let position = pool.positions.get(&position_id).expect(PST0).clone();
    //     let token = pool.token1.clone();
    //     let collateral = position.total_locked;
    //     let borrowed = (BORROW_RATIO * collateral).round() as u128; // health factor 1.25
    //     let liquidation_price = position.get_liquidation_price(borrowed as f64);
    //     self.increase_balance(&account_id, &token, borrowed);
    //     let mut reserve = self.reserves.get(&token).expect(RSR0);
    //     reserve.borrowed += borrowed;
    //     assert!(
    //         reserve.deposited >= reserve.borrowed,
    //         "{}",
    //         borrow_error(&token, reserve.borrowed, reserve.deposited)
    //     );
    //     self.reserves.insert(&token, &reserve);
    //     let borrow = Borrow {
    //         owner_id: account_id,
    //         asset0: String::new(),
    //         asset1: token.to_string(),
    //         borrowed0: 0,
    //         borrowed1: borrowed,
    //         collateral: collateral as u128,
    //         position_id,
    //         pool_id,
    //         last_update_timestamp: env::block_timestamp(),
    //         apr: APR_BORROW,
    //         leverage: None,
    //         fees: 0,
    //         liquidation_price,
    //     };
    //     self.borrows.insert(&self.borrows_number, &borrow);
    //     self.borrows_number += 1;
    //     self.nft_transfer(
    //         env::current_account_id(),
    //         position_id.to_string(),
    //         None,
    //         None,
    //     );
    //     borrowed.into()
    // }

    #[payable]
    pub fn supply_collateral_and_borrow(
        &mut self,
        pool_id: usize,
        position_id: u128,
        leverage: f64,
    ) {
        assert!(leverage > 1.0);
        let account_id = env::predecessor_account_id();
        let pool = &mut self.pools[pool_id];
        let token0 = pool.token0.clone();
        let token1 = pool.token1.clone();
        let position = pool.positions.get(&position_id).expect(PST0).clone();
        let borrowed0 = (position.token0_locked * (leverage - 1.0)) as u128;
        let borrowed1 = (position.token1_locked * (leverage - 1.0)) as u128;

        let mut reserve = self.reserves.get(&token0).expect(RSR0);
        reserve.borrowed += borrowed0;
        assert!(reserve.deposited >= reserve.borrowed);
        self.reserves.insert(&token0, &reserve);

        let mut reserve = self.reserves.get(&token1).expect(RSR0);
        reserve.borrowed += borrowed1;
        assert!(reserve.deposited >= reserve.borrowed);
        self.reserves.insert(&token1, &reserve);

        let mut position = pool.positions.get(&position_id).expect(PST0).clone();
        position.add_liquidity(
            Some(U128::from(borrowed0)),
            None,
            pool.sqrt_price,
        );
        let liquidation_price = position.get_liquidation_price(borrowed0 as f64, borrowed1 as f64, LTV_MAX);
        pool.positions.insert(position_id, position);

        let borrow = Borrow {
            id: self.borrows_number,
            owner_id: account_id,
            asset0: token0,
            asset1: token1,
            borrowed0,
            borrowed1,
            position_id,
            pool_id,
            last_update_timestamp: env::block_timestamp(),
            apr: APR_BORROW,
            leverage: leverage,
            fees: 0,
            liquidation_price,
        };
        self.borrows.insert(&self.borrows_number, &borrow);
        self.borrows_number += 1;
        // Make sure the loan is sufficiently overcollateralized
        let health_factor = self.get_borrow_health_factor(borrow.id);
        assert!(health_factor >= 1.0);

        // commented out for now because when NFT is tranferred away from the owner
        // other functions will fail without it, including return_collateral_and_repay()
        //
        // self.nft_transfer(
        //     env::current_account_id(),
        //     position_id.to_string(),
        //     None,
        //     None,
        // );
    }

    pub fn return_collateral_and_repay(&mut self, borrow_id: u128) {
        let account_id = env::predecessor_account_id();
        let borrow = self.borrows.remove(&borrow_id).expect(BRR0);
        let pool = &self.pools[borrow.pool_id];
        let position = pool.positions.get(&borrow.position_id).expect(PST0);
        assert_eq!(account_id, borrow.owner_id);
        let mut reserve = self.reserves.get(&borrow.asset0).expect(RSR0);
        reserve.borrowed -= borrow.borrowed0;
        self.reserves.insert(&borrow.asset0, &reserve);
        let mut reserve = self.reserves.get(&borrow.asset1).expect(RSR0);
        reserve.borrowed -= borrow.borrowed1;
        self.reserves.insert(&borrow.asset1, &reserve);
        self.remove_liquidity(
            borrow.pool_id,
            borrow.position_id,
            None,
            Some(U128::from(borrow.borrowed1 + borrow.fees)),
        );
        // ext_self::nft_transfer(
        //     account_id,
        //     borrow.position_id.to_string(),
        //     None,
        //     None,
        //     &env::current_account_id(),
        //     0,
        //     10 * TGAS,
        // );
    }

    pub fn get_liquidation_list(&self) -> Vec<BorrowId> {
        self.borrows
            .iter()
            .filter(|(id, _)| self.get_borrow_health_factor(*id) < 1.0)
            .map(|(id, _)| id)
            .collect()
    }

    pub fn get_borrows_by_account(&self, account_id: AccountId) -> Vec<Borrow> {
        self.borrows
            .iter()
            .filter(|(_, borrow)| borrow.owner_id == account_id)
            .map(|(_, borrow)| borrow)
            .collect()
    }

    pub fn get_liquidation_price(
        &self,
        pool_id: usize,
        token0_liquidity: Option<U128>,
        token1_liquidity: Option<U128>,
        lower_bound_price: f64,
        upper_bound_price: f64,
        borrowed0: f64,
        borrowed1: f64,
    ) -> (f64, f64) {
        self.assert_pool_exists(pool_id);
        let pool = &self.pools[pool_id];
        let position = Position::new(
            String::new(),
            token0_liquidity.map(|v| U128(v.0 + borrowed0 as u128)).or(None),
            token1_liquidity.map(|v| U128(v.0 + borrowed1 as u128)).or(None),
            lower_bound_price,
            upper_bound_price,
            pool.sqrt_price,
        );
        position.get_liquidation_price(borrowed0, borrowed1, LTV_MAX)
    }

    pub fn get_max_leverage(
        &self,
        pool_id: usize,
        lower_bound_price: f64,
        upper_bound_price: f64,
    ) -> f64 {
        self.assert_pool_exists(pool_id);
        let pool = &self.pools[pool_id];
        let pb = upper_bound_price;
        let rpa = lower_bound_price.sqrt();
        let rpb = upper_bound_price.sqrt();
        let rp = pool.sqrt_price;
        if rp > rpa && rp < rpb {
            1.0 / (1.0 - LTV_MAX / f64::max(
                ((rp - rpa) / (rpb - rpa)) * (rp / rpa) + ((rpb - rp) / (rpb - rpa)) * (rpa / rp),
                (pb / rp - rpb + rp - rpa) / (rpb - rpa)
            ))
        } else {
            1.0 / (1.0 - LTV_MAX * rpa / rpb)
        }
    }

    pub fn get_borrow_health_factor(&self, borrow_id: BorrowId) -> f64 {
        let borrow = self.borrows.get(&borrow_id).expect(BRR0);
        let pool = &self.pools[borrow.pool_id];
        let position = pool.positions.get(&borrow.position_id).unwrap();
        let price = pool.sqrt_price * pool.sqrt_price;
        let ltv = (borrow.borrowed0 as f64 * price + borrow.borrowed1 as f64) / (position.total_locked as f64);
        LTV_MAX / ltv
    }

    pub fn liquidate(&mut self, borrow_id: BorrowId) {
        let account_id = env::predecessor_account_id();
        let borrow = self.borrows.remove(&borrow_id).expect(BRR0);
        let pool = &self.pools[borrow.pool_id];
        let position = pool.positions.get(&borrow.position_id).unwrap();
        let health_factor = self.get_borrow_health_factor(borrow_id);
        assert!(health_factor < 1.0);
        let discount = (1.0 + health_factor) / 2.0;
        let discounted_collateral_sum = (position.total_locked * discount / borrow.leverage) as u128;
        self.decrease_balance(&account_id, &borrow.asset1, discounted_collateral_sum);
        if let leverage = borrow.leverage {
            let pool = &mut self.pools[borrow.pool_id];
            let mut position = pool.positions.get(&borrow.position_id).unwrap().clone();
            position.remove_liquidity(
                Some(U128::from(
                    (position.token0_locked * (leverage - 1.0) / leverage) as u128,
                )),
                None,
                pool.sqrt_price,
            );
            pool.positions.insert(borrow.position_id, position);
        }
        ext_self::nft_transfer(
            account_id,
            borrow.position_id.to_string(),
            None,
            None,
            &env::current_account_id(),
            0,
            10 * TGAS,
        );
    }
}
