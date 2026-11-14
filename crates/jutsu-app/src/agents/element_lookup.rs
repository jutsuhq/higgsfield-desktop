//! The account's reference elements as agents name them (`AgentElementLookup.swift`): `elements` lists them with
//! References › From Higgsfield's own source, and `create_video`, `quote` and `create_image` take them by name.

use jutsu_core::{AccountElement, AccountElements, ElementCategory, HistoryCursor, LocalImage, PortError};

/// Pages read for one answer, so a name that matches nothing never walks a whole account.
pub const MAX_PAGES: usize = 20;

/// Whether `element` has every word of `query` in its name, in any case, and is of `category`.
pub fn matches(element: &AccountElement, query: &str, category: Option<ElementCategory>) -> bool {
    if let Some(category) = category
        && element.category != Some(category)
    {
        return false;
    }
    let name = element.name.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| name.contains(word))
}

/// What a name finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Element(Box<AccountElement>),
    /// Several elements have the name: the agent names one by its exact name, or the person renames one.
    Ambiguous(usize),
    Missing,
    /// Not every element was read (`MAX_PAGES`): one of the name, or a second one, may come later.
    Unchecked,
}

/// Where a page of `elements` ended: the account's cursor of the page it ended in (`None` for the first) and how many
/// of that page's elements came before. As text, the account's cursor alone when none did, else `<skip>@<cursor>`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Position {
    page: Option<HistoryCursor>,
    skip: usize,
}

impl Position {
    fn parse(text: &str) -> Self {
        if let Some((skip, page)) = text.split_once('@')
            && let Ok(skip) = skip.parse::<usize>()
            && skip > 0
        {
            return Self {
                page: (!page.is_empty()).then(|| HistoryCursor::new(page)),
                skip,
            };
        }
        Self {
            page: Some(HistoryCursor::new(text)),
            skip: 0,
        }
    }

    fn text(&self) -> String {
        let page = self
            .page
            .as_ref()
            .map(|page| page.value.clone())
            .unwrap_or_default();
        if self.skip > 0 {
            format!("{}@{page}", self.skip)
        } else {
            page
        }
    }
}

/// One answer of `elements`: at most `limit` elements that match `query` and `category`, read page after page from
/// `cursor` until there are `limit`, the last page or `MAX_PAGES`, and the cursor to go on from, `None` after the last
/// element. A page cut short hands out a cursor into it, so no element is skipped or repeated.
pub async fn page(
    source: &dyn AccountElements,
    query: &str,
    category: Option<ElementCategory>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<(Vec<AccountElement>, Option<String>), PortError> {
    let limit = limit.max(1);
    let mut position = cursor.map_or(Position { page: None, skip: 0 }, Position::parse);
    let mut items = Vec::new();
    for _ in 0..MAX_PAGES {
        let read = source.elements(position.page.clone()).await?;
        let count = read.items.len();
        for (index, element) in read.items.into_iter().enumerate().skip(position.skip) {
            if !matches(&element, query, category) {
                continue;
            }
            items.push(element);
            if items.len() < limit {
                continue;
            }
            if index + 1 < count {
                let inside = Position {
                    page: position.page.clone(),
                    skip: index + 1,
                };
                return Ok((items, Some(inside.text())));
            }
            let next = read.next_cursor.map(|page| {
                Position {
                    page: Some(page),
                    skip: 0,
                }
                .text()
            });
            return Ok((items, next));
        }
        let Some(next) = read.next_cursor else {
            return Ok((items, None));
        };
        position = Position {
            page: Some(next),
            skip: 0,
        };
    }
    Ok((items, Some(position.text())))
}

/// The account's elements, page after page until the last or `MAX_PAGES`, and whether that was all of them: a name is
/// known to be one element only once they are all read, as a second one of that name may be on a later page.
pub async fn all(source: &dyn AccountElements) -> Result<(Vec<AccountElement>, bool), PortError> {
    let mut elements = Vec::new();
    let mut cursor = None;
    for _ in 0..MAX_PAGES {
        let read = source.elements(cursor).await?;
        elements.extend(read.items);
        cursor = read.next_cursor;
        if cursor.is_none() {
            return Ok((elements, true));
        }
    }
    Ok((elements, false))
}

/// The element named `name`, in any case, among `elements`; `is_complete` false when they aren't all the account's,
/// so only two of the name are certain.
pub fn find(name: &str, elements: &[AccountElement], is_complete: bool) -> Found {
    let wanted = name.trim().to_lowercase();
    let named: Vec<&AccountElement> = elements
        .iter()
        .filter(|element| element.name.trim().to_lowercase() == wanted)
        .collect();
    if named.len() > 1 {
        return Found::Ambiguous(named.len());
    }
    if !is_complete {
        return Found::Unchecked;
    }
    named.first().map_or(Found::Missing, |element| {
        Found::Element(Box::new((*element).clone()))
    })
}

/// A named element's images as files on this computer, in the element's order, as a request sends them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementImages {
    pub name: String,
    pub images: Vec<LocalImage>,
}

/// One image a request sends, by the tag its prompt names it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// "@Image1", "@Image2"…
    pub tag: String,
    /// The element's name; `None` for the agent's own file.
    pub element: Option<String>,
    pub image: LocalImage,
}

/// Each image of a request, in the order it is sent, with the element it belongs to: the elements' images first, in
/// the order named, then the agent's own files.
pub fn references(elements: &[ElementImages], files: &[LocalImage]) -> Vec<Reference> {
    elements
        .iter()
        .flat_map(|element| {
            element
                .images
                .iter()
                .map(|image| (Some(element.name.clone()), image.clone()))
        })
        .chain(files.iter().map(|image| (None, image.clone())))
        .enumerate()
        .map(|(index, (element, image))| Reference {
            tag: format!("@Image{}", index + 1),
            element,
            image,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use jutsu_core::{AccountElementPage, RemoteAsset};

    use super::*;

    /// Elements a page at a time, recording the cursors asked for.
    struct PagedElements {
        pages: Vec<AccountElementPage>,
        asked: Mutex<Vec<Option<HistoryCursor>>>,
    }

    impl PagedElements {
        fn new(pages: Vec<AccountElementPage>) -> Self {
            Self {
                pages,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<Option<HistoryCursor>> {
            self.asked.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AccountElements for PagedElements {
        fn owner(&self) -> Option<String> {
            Some("user_1".to_owned())
        }
        async fn elements(&self, after: Option<HistoryCursor>) -> Result<AccountElementPage, PortError> {
            let mut asked = self.asked.lock().unwrap();
            asked.push(after);
            Ok(self.pages[asked.len() - 1].clone())
        }
        async fn element_images(&self, element: AccountElement) -> Result<AccountElement, PortError> {
            Ok(element)
        }
        async fn create_element(
            &self,
            name: String,
            category: ElementCategory,
            _images: Vec<RemoteAsset>,
        ) -> Result<AccountElement, PortError> {
            Ok(element("el-new", &name, category))
        }
    }

    fn element(id: &str, name: &str, category: ElementCategory) -> AccountElement {
        AccountElement {
            id: id.to_owned(),
            name: name.to_owned(),
            category: Some(category),
            thumbnail_url: None,
            images: Vec::new(),
            status: None,
        }
    }

    fn mara() -> AccountElement {
        element("el-1", "Mara Vance", ElementCategory::Character)
    }

    fn roof() -> AccountElement {
        element("el-2", "Roof", ElementCategory::Environment)
    }

    fn twin() -> AccountElement {
        AccountElement {
            id: "el-3".to_owned(),
            ..mara()
        }
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "the cursors the fake is asked with are optional"
    )]
    fn cursor(value: &str) -> Option<HistoryCursor> {
        Some(HistoryCursor::new(value))
    }

    fn page_of(items: Vec<AccountElement>, next: Option<&str>) -> AccountElementPage {
        AccountElementPage {
            items,
            next_cursor: next.map(HistoryCursor::new),
        }
    }

    #[test]
    fn every_word_of_the_query_is_in_the_name_in_any_case() {
        assert!(matches(&mara(), "vance MARA", None));
        assert!(matches(&mara(), "", Some(ElementCategory::Character)));
        assert!(!matches(&mara(), "mara", Some(ElementCategory::Prop)));
        assert!(!matches(&roof(), "mara", None));
    }

    #[test]
    fn a_name_is_found_whole_in_any_case_and_two_of_it_are_ambiguous() {
        assert_eq!(
            find(" mara vance", &[roof(), mara()], true),
            Found::Element(Box::new(mara()))
        );
        assert_eq!(find("Mara", &[roof(), mara()], true), Found::Missing);
        assert_eq!(find("Mara Vance", &[mara(), twin()], true), Found::Ambiguous(2));
    }

    /// A twin on a later page still makes the name ambiguous: every page is read before a name is decided.
    #[tokio::test]
    async fn every_page_is_read_so_a_twin_on_a_later_page_is_found() {
        let source = PagedElements::new(vec![
            page_of(vec![mara()], Some("2")),
            page_of(vec![roof()], Some("3")),
            page_of(vec![twin()], None),
        ]);
        let (elements, is_complete) = all(&source).await.unwrap();
        assert_eq!(source.asked(), [None, cursor("2"), cursor("3")]);
        assert!(is_complete);
        assert_eq!(find("Mara Vance", &elements, true), Found::Ambiguous(2));
        assert_eq!(find("roof", &elements, true), Found::Element(Box::new(roof())));
    }

    /// A page of `elements` holds at most `limit`; one that ends inside a page of the account's hands out a cursor
    /// into it, and the next page starts with the element after.
    #[tokio::test]
    async fn a_page_holds_at_most_the_limit_and_goes_on_where_it_stopped() {
        let pages = [
            page_of(vec![roof()], Some("3")),
            page_of(vec![mara(), roof(), twin()], Some("4")),
            page_of(vec![mara()], None),
        ];
        // The fake answers page after page whatever the cursor: the second call reads the middle page again.
        let source = PagedElements::new(vec![
            pages[0].clone(),
            pages[1].clone(),
            pages[1].clone(),
            pages[2].clone(),
        ]);
        let (first, next) = page(&source, "mara", None, 1, Some("2")).await.unwrap();
        assert_eq!(first, [mara()]);
        assert_eq!(source.asked(), [cursor("2"), cursor("3")]);
        let next = next.unwrap();

        let (second, after) = page(&source, "mara", None, 1, Some(&next)).await.unwrap();
        assert_eq!(second, [twin()]);
        // The page's last element: the cursor is the account's next page.
        assert_eq!(after.as_deref(), Some("4"));
        assert_eq!(source.asked().last(), Some(&cursor("3")));

        let (rest, end) = page(&source, "mara", None, 5, after.as_deref()).await.unwrap();
        assert_eq!(rest, [mara()]);
        assert_eq!(end, None);
    }

    /// A query that matches nothing stops after `MAX_PAGES`, with the cursor to go on from; looking a name up in that
    /// many pages can't tell one element of it from two, so it is unchecked.
    #[tokio::test]
    async fn a_lookup_stops_after_max_pages() {
        let pages: Vec<_> = (1..=45)
            .map(|index| page_of(vec![roof()], Some(&(index + 1).to_string())))
            .collect();
        let source = PagedElements::new(pages.clone());
        let (items, next) = page(&source, "mara", None, 5, None).await.unwrap();
        assert_eq!(items.len(), 0);
        assert_eq!(source.asked().len(), MAX_PAGES);
        assert_eq!(next, Some((MAX_PAGES + 1).to_string()));

        let source = PagedElements::new(pages);
        let (elements, is_complete) = all(&source).await.unwrap();
        assert!(!is_complete);
        assert_eq!(find("roof", &elements, is_complete), Found::Ambiguous(20));
        assert_eq!(find("Mara Vance", &elements, false), Found::Unchecked);
        assert_eq!(find("Mara Vance", &[mara()], false), Found::Unchecked);
    }
}
