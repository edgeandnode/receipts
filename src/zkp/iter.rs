use super::receipt::*;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    DuplicateID,
    InvalidID,
}

/// Returns an iterator with validated, ordered receipts suitable for
/// consumption in the zkp.
pub fn rows(mut receipts: Vec<Receipt>) -> Result<RowIterator, Error> {
    // Sorted in reverse order so that they are in order
    // when popped.
    receipts.sort_by(|a, b| b.id.cmp(&a.id));

    // Verify that no items are > max
    // Only the first item has to be checked,
    // because it is the largest
    if let Some(last) = receipts.first() {
        if last.id > Receipt::MAX_ID {
            return Err(Error::InvalidID);
        }
    }

    // Verify that no items are identical
    // by checking adjacent ids
    for window in receipts.windows(2) {
        if window[0].id == window[1].id {
            return Err(Error::DuplicateID);
        }
    }

    Ok(RowIterator { id: 0, receipts })
}

#[derive(Eq, PartialEq, Debug)]
pub struct RowIterator {
    id: ID,
    receipts: Vec<Receipt>,
}

impl Iterator for RowIterator {
    type Item = Receipt;
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.id;
        if id == Receipt::MAX_ID {
            return None;
        }
        self.id += 1;

        if let Some(receipt) = self.receipts.last() {
            if id == receipt.id {
                return self.receipts.pop();
            }
        }

        Some(Receipt::null(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn receipts_are_ordered_and_holes_filled() {
        let a = Receipt {
            id: 0,
            payment_amount: 0,
            signature: (),
        };
        let b = Receipt {
            id: 2,
            payment_amount: 0,
            signature: (),
        };

        let check = |receipts| {
            let mut receipts = rows(receipts).unwrap();
            assert_eq!(receipts.next(), Some(a.clone()));
            assert_eq!(receipts.next(), Some(Receipt::null(1)));
            assert_eq!(receipts.next(), Some(b.clone()));
            assert_eq!(receipts.next(), Some(Receipt::null(3)));
            assert_eq!(receipts.count(), Receipt::MAX_ID as usize - 4);
        };

        // Try two different initial orderings and verify that they are in the correct
        // order each time.
        check(vec![a.clone(), b.clone()]);
        check(vec![b.clone(), a.clone()]);
    }

    #[test]
    pub fn duplicate_ids_rejected() {
        let a = Receipt {
            id: 10,
            payment_amount: 2,
            signature: (),
        };
        let b = Receipt {
            id: 10,
            payment_amount: 3,
            signature: (),
        };

        let receipts = vec![a, b];
        assert_eq!(rows(receipts).unwrap_err(), Error::DuplicateID);
    }

    #[test]
    pub fn max_id_rejected() {
        let a = Receipt {
            id: Receipt::MAX_ID.checked_add(1).unwrap(),
            payment_amount: 0,
            signature: (),
        };
        let b = Receipt {
            id: 5,
            payment_amount: 0,
            signature: (),
        };

        let receipts = vec![a.clone(), b.clone()];
        assert_eq!(rows(receipts).unwrap_err(), Error::InvalidID);

        let receipts = vec![b, a];
        assert_eq!(rows(receipts).unwrap_err(), Error::InvalidID);
    }
}
