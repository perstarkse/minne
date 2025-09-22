use serde::Serialize;

/// Metadata describing a paginated collection.
#[derive(Debug, Clone, Serialize)]
pub struct Pagination {
    pub current_page: usize,
    pub per_page: usize,
    pub total_items: usize,
    pub total_pages: usize,
    pub has_previous: bool,
    pub has_next: bool,
    pub previous_page: Option<usize>,
    pub next_page: Option<usize>,
    pub start_index: usize,
    pub end_index: usize,
}

impl Pagination {
    pub fn new(
        current_page: usize,
        per_page: usize,
        total_items: usize,
        total_pages: usize,
        page_len: usize,
    ) -> Self {
        let has_pages = total_pages > 0;
        let has_previous = has_pages && current_page > 1;
        let has_next = has_pages && current_page < total_pages;
        let offset = if has_pages {
            per_page.saturating_mul(current_page.saturating_sub(1))
        } else {
            0
        };
        let start_index = if page_len == 0 { 0 } else { offset + 1 };
        let end_index = if page_len == 0 { 0 } else { offset + page_len };

        Self {
            current_page,
            per_page,
            total_items,
            total_pages,
            has_previous,
            has_next,
            previous_page: if has_previous {
                Some(current_page - 1)
            } else {
                None
            },
            next_page: if has_next {
                Some(current_page + 1)
            } else {
                None
            },
            start_index,
            end_index,
        }
    }
}

/// Returns the items for the requested page along with pagination metadata.
pub fn paginate_items<T>(
    items: Vec<T>,
    requested_page: Option<usize>,
    per_page: usize,
) -> (Vec<T>, Pagination) {
    let per_page = per_page.max(1);
    let total_items = items.len();
    let total_pages = if total_items == 0 {
        0
    } else {
        ((total_items - 1) / per_page) + 1
    };

    let mut current_page = requested_page.unwrap_or(1);
    if current_page == 0 {
        current_page = 1;
    }
    if total_pages > 0 {
        current_page = current_page.min(total_pages);
    } else {
        current_page = 1;
    }

    let offset = if total_pages == 0 {
        0
    } else {
        per_page.saturating_mul(current_page - 1)
    };

    let page_items: Vec<T> = items.into_iter().skip(offset).take(per_page).collect();
    let page_len = page_items.len();
    let pagination = Pagination::new(current_page, per_page, total_items, total_pages, page_len);

    (page_items, pagination)
}

#[cfg(test)]
mod tests {
    use super::paginate_items;

    #[test]
    fn paginates_basic_case() {
        let items: Vec<_> = (1..=25).collect();
        let (page, meta) = paginate_items(items, Some(2), 10);

        assert_eq!(page, vec![11, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
        assert_eq!(meta.current_page, 2);
        assert_eq!(meta.total_pages, 3);
        assert!(meta.has_previous);
        assert!(meta.has_next);
        assert_eq!(meta.previous_page, Some(1));
        assert_eq!(meta.next_page, Some(3));
        assert_eq!(meta.start_index, 11);
        assert_eq!(meta.end_index, 20);
    }

    #[test]
    fn handles_empty_items() {
        let items: Vec<u8> = vec![];
        let (page, meta) = paginate_items(items, Some(3), 10);

        assert!(page.is_empty());
        assert_eq!(meta.current_page, 1);
        assert_eq!(meta.total_pages, 0);
        assert!(!meta.has_previous);
        assert!(!meta.has_next);
        assert_eq!(meta.start_index, 0);
        assert_eq!(meta.end_index, 0);
    }

    #[test]
    fn clamps_page_to_bounds() {
        let items: Vec<_> = (1..=5).collect();
        let (page, meta) = paginate_items(items, Some(10), 2);

        assert_eq!(page, vec![5]);
        assert_eq!(meta.current_page, 3);
        assert_eq!(meta.total_pages, 3);
        assert_eq!(meta.has_next, false);
        assert_eq!(meta.has_previous, true);
        assert_eq!(meta.start_index, 5);
        assert_eq!(meta.end_index, 5);
    }
}
