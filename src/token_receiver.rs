use crate::Contract;
use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::json_types::ValidAccountId;
use near_sdk::serde_json;
use near_sdk::{env, json_types::U128, near_bindgen, PromiseOrValue};

use crate::action::Action;
use crate::*;

/// Message parameters to receive via token function call.
#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
#[serde(untagged)]
enum TokenReceiverMessage {
    /// Alternative to deposit + execute actions call.
    Execute { actions: Vec<Action> },
}

#[derive(Clone, Serialize, BorshDeserialize, BorshSerialize, PartialEq)]
#[serde(crate = "near_sdk::serde")]
pub struct OpenPositionRequest {
    pub owner_id: AccountId,
    pub pool_id: usize,
    pub token0_liquidity: Option<U128>,
    pub token1_liquidity: Option<U128>,
    pub lower_bound_price: f64,
    pub upper_bound_price: f64,
}

impl Contract {
    // may be needed to change predecessor_id -> signer_id in some method
    fn internal_execute(&mut self, token_in: AccountId, actions: &[Action]) {
        for action in actions {
            match action {
                Action::Swap(action) => {
                    assert_eq!(token_in, action.token_in);
                    self.swap(
                        action.pool_id,
                        &action.token_in,
                        action.amount_in,
                        &action.token_out,
                    );
                }
                Action::Withdraw(action) => {
                    self.withdraw(&action.token, action.amount);
                }
                Action::MultihopeSwap(action) => {
                    self.swap_multihope(&action.token_in, action.amount_in, &action.token_out);
                }
                Action::OpenPosition(action) => {
                    let account = env::signer_account_id();
                    if let Some(request) = self.open_position_requests.get(&action.request_id) {
                        let mut request = request.clone();
                        assert_eq!(account, request.owner_id);
                        assert_eq!(action.pool_id, request.pool_id);
                        assert_eq!(action.lower_bound_price, request.lower_bound_price);
                        assert_eq!(action.upper_bound_price, request.upper_bound_price);
                        if request.token0_liquidity.is_none() {
                            request.token0_liquidity = action.token0_liquidity;
                        } else {
                            request.token1_liquidity = action.token1_liquidity;
                        }
                        self.open_position(
                            request.pool_id,
                            request.token0_liquidity,
                            request.token1_liquidity,
                            request.lower_bound_price,
                            request.upper_bound_price,
                        );
                        self.open_position_requests.remove(&action.request_id);
                    } else {
                        assert!(
                            action.token0_liquidity.is_some() ^ action.token1_liquidity.is_some(),
                            "{}",
                            PST5
                        );
                        let request = OpenPositionRequest {
                            owner_id: account,
                            pool_id: action.pool_id,
                            token0_liquidity: action.token0_liquidity,
                            token1_liquidity: action.token1_liquidity,
                            lower_bound_price: action.lower_bound_price,
                            upper_bound_price: action.upper_bound_price,
                        };
                        self.open_position_requests
                            .insert(action.request_id, request);
                    }
                }
                Action::AddLiquidity(action) => {
                    self.add_liquidity(
                        action.pool_id,
                        action.position_id,
                        action.token0_liquidity,
                        action.token1_liquidity,
                    );
                }
                Action::CreateDeposit(action) => {
                    self.create_deposit(&action.asset, action.amount);
                }
                Action::ReturnCollateralAndRepay(action) => {
                    self.return_collateral_and_repay(action.borrow_id);
                }
                Action::Liquidate(action) => {
                    self.liquidate(action.borrow_id);
                }
            }
        }
    }
}

#[near_bindgen]
impl FungibleTokenReceiver for Contract {
    #[allow(unreachable_code)]
    #[allow(unused_variables)]
    fn ft_on_transfer(
        &mut self,
        sender_id: ValidAccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        let token_in = env::predecessor_account_id();
        self.deposit_ft(&sender_id.into(), &token_in, amount.into());
        if msg == "".to_string() {
            return PromiseOrValue::Value(U128(0));
        }
        // instant swap
        let message = serde_json::from_str::<TokenReceiverMessage>(&msg).expect("Wrong msg format");
        match message {
            TokenReceiverMessage::Execute { actions } => {
                self.internal_execute(token_in, &actions);
                return PromiseOrValue::Value(U128(0));
            }
        }
        PromiseOrValue::Value(U128(0))
    }
}

#[cfg(test)]
mod test {
    use crate::action::{SwapAction, WithdrawAction};

    use super::*;

    #[test]
    fn message_test() {
        let swap_action = Action::Swap(SwapAction {
            pool_id: 0,
            token_in: "token_in.testnet".to_string(),
            amount_in: U128(1000),
            token_out: "token_out.testnet".to_string(),
        });
        let withdraw_action = Action::Withdraw(WithdrawAction {
            token: "token_in.testnet".to_string(),
            amount: U128(1000),
        });
        let token_receiver_message = TokenReceiverMessage::Execute {
            actions: vec![swap_action, withdraw_action],
        };
        let message: String = serde_json::to_value(&token_receiver_message)
            .unwrap()
            .to_string();
        println!("message is: {}", message);
    }
}
