use crate::EscrowError;

pub const MAX_SINGLE_AMOUNT_STROOPS: i128 = 100_000_000_000_000_000;

pub fn validate_single_amount(amount: i128) -> Result<(), EscrowError> {
    if amount <= 0 {
        return Err(EscrowError::AmountMustBePositive);
    }
    Ok(())
}

pub fn validate_amount_array(amounts: &[i128]) -> Result<i128, EscrowError> {
    let mut total = 0i128;
    for amount in amounts {
        validate_single_amount(*amount)?;
        total = total
            .checked_add(*amount)
            .ok_or(EscrowError::PotentialOverflow)?;
    }
    Ok(total)
}

pub fn validate_milestone_amounts(
    amounts: &[i128],
    max_total: i128,
) -> Result<(), EscrowError> {
    for amount in amounts {
        validate_single_amount(*amount)?;
    }
    let total = validate_amount_array(amounts)?;
    if total > max_total {
        return Err(EscrowError::TotalCapExceeded);
    }
    Ok(())
}

pub fn accumulate_amounts<I>(amounts: I) -> Result<i128, EscrowError>
where
    I: Iterator<Item = i128>,
{
    let mut total = 0i128;
    for amount in amounts {
        total = total
            .checked_add(amount)
            .ok_or(EscrowError::PotentialOverflow)?;
    }
    Ok(total)
}

pub fn safe_add_amounts(a: i128, b: i128) -> Option<i128> {
    a.checked_add(b)
}

pub fn safe_subtract_amounts(a: i128, b: i128) -> Option<i128> {
    a.checked_sub(b)
}

pub fn validate_deposit_amount(amount: i128) -> Result<(), EscrowError> {
    validate_single_amount(amount)
}

pub fn checked_available_balance(
    funded_amount: i128,
    released_amount: i128,
    refunded_amount: i128,
) -> Result<i128, EscrowError> {
    let balance = funded_amount
        .checked_sub(released_amount)
        .ok_or(EscrowError::AccountingInvariantViolated)?;
    let balance = balance
        .checked_sub(refunded_amount)
        .ok_or(EscrowError::AccountingInvariantViolated)?;
    Ok(balance)
}