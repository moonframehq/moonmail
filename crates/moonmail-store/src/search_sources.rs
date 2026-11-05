use {
    crate::{Error, Result, SavedSearch},
    moonmail_core::MailPredicate as P,
    std::collections::HashSet,
};

/// No sources means all live mail. Otherwise intersect the local predicate with
/// the union of its source searches. References are evaluated from one metadata
/// snapshot; never store a frozen list of matching message IDs.
pub(crate) fn resolve(id: i64, searches: &[SavedSearch]) -> Result<P> {
    let predicate = expand(id, searches, &mut HashSet::new(), &mut 64, 0)?;
    crate::predicates::compile(&predicate)?;
    Ok(predicate)
}

fn charge(predicate: &P, budget: &mut usize) -> Result<()> {
    *budget = budget.checked_sub(1).ok_or(Error::Invalid(
        "Combined saved search exceeds 64 conditions",
    ))?;
    match predicate {
        P::All(children) | P::Any(children) => {
            for child in children {
                charge(child, budget)?;
            }
        },
        P::Not(child) => charge(child, budget)?,
        _ => {},
    }
    Ok(())
}

fn expand(
    id: i64,
    searches: &[SavedSearch],
    path: &mut HashSet<i64>,
    budget: &mut usize,
    depth: usize,
) -> Result<P> {
    if depth > 16 {
        return Err(Error::Invalid(
            "Saved search source nesting exceeds 16 levels",
        ));
    }
    if !path.insert(id) {
        return Err(Error::Invalid("Saved search sources form a cycle"));
    }
    let search = searches
        .iter()
        .find(|search| search.id == id)
        .ok_or(Error::Invalid("Saved search source no longer exists"))?;
    charge(&search.predicate, budget)?;
    let result = if search.sources.is_empty() {
        search.predicate.clone()
    } else {
        *budget = budget.checked_sub(2).ok_or(Error::Invalid(
            "Combined saved search exceeds 64 conditions",
        ))?;
        let sources = search
            .sources
            .iter()
            .map(|id| expand(*id, searches, path, budget, depth + 1))
            .collect::<Result<Vec<_>>>()?;
        P::All(vec![search.predicate.clone(), P::Any(sources)])
    };
    path.remove(&id);
    Ok(result)
}
