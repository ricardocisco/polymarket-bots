// src/cities.rs
//! Lista fixa de cidades usada pelo bot principal, setup e backtest.
//!
//! O `sim_monitor` já faz descoberta dinâmica via `tag_slug=temperature`.
//! Este arquivo existe para os fluxos que ainda percorrem slugs manualmente.

/// Keyword de slug para descoberta manual.
pub struct CitySlug {
    pub name: &'static str,
    pub keyword: &'static str,
}

/// Conjunto fixo de 44 cidades informado para os mercados de 5 Apr 2026.
pub fn all_slugs() -> Vec<CitySlug> {
    vec![
        CitySlug {
            name: "Shanghai",
            keyword: "highest-temperature-in-shanghai",
        },
        CitySlug {
            name: "New York",
            keyword: "highest-temperature-in-nyc",
        },
        CitySlug {
            name: "London",
            keyword: "highest-temperature-in-london",
        },
        CitySlug {
            name: "Seoul",
            keyword: "highest-temperature-in-seoul",
        },
        CitySlug {
            name: "Tokyo",
            keyword: "highest-temperature-in-tokyo",
        },
        CitySlug {
            name: "Wellington",
            keyword: "highest-temperature-in-wellington",
        },
        CitySlug {
            name: "Atlanta",
            keyword: "highest-temperature-in-atlanta",
        },
        CitySlug {
            name: "Chicago",
            keyword: "highest-temperature-in-chicago",
        },
        CitySlug {
            name: "Toronto",
            keyword: "highest-temperature-in-toronto",
        },
        CitySlug {
            name: "Singapore",
            keyword: "highest-temperature-in-singapore",
        },
        CitySlug {
            name: "Beijing",
            keyword: "highest-temperature-in-beijing",
        },
        CitySlug {
            name: "Los Angeles",
            keyword: "highest-temperature-in-los-angeles",
        },
        CitySlug {
            name: "Seattle",
            keyword: "highest-temperature-in-seattle",
        },
        CitySlug {
            name: "Dallas",
            keyword: "highest-temperature-in-dallas",
        },
        CitySlug {
            name: "Hong Kong",
            keyword: "highest-temperature-in-hong-kong",
        },
        CitySlug {
            name: "Munich",
            keyword: "highest-temperature-in-munich",
        },
        CitySlug {
            name: "Ankara",
            keyword: "highest-temperature-in-ankara",
        },
        CitySlug {
            name: "Milan",
            keyword: "highest-temperature-in-milan",
        },
        CitySlug {
            name: "Miami",
            keyword: "highest-temperature-in-miami",
        },
        CitySlug {
            name: "Taipei",
            keyword: "highest-temperature-in-taipei",
        },
        CitySlug {
            name: "Madrid",
            keyword: "highest-temperature-in-madrid",
        },
        CitySlug {
            name: "Paris",
            keyword: "highest-temperature-in-paris",
        },
        CitySlug {
            name: "Shenzhen",
            keyword: "highest-temperature-in-shenzhen",
        },
        CitySlug {
            name: "Moscow",
            keyword: "highest-temperature-in-moscow",
        },
        CitySlug {
            name: "Warsaw",
            keyword: "highest-temperature-in-warsaw",
        },
        CitySlug {
            name: "Austin",
            keyword: "highest-temperature-in-austin",
        },
        CitySlug {
            name: "Lucknow",
            keyword: "highest-temperature-in-lucknow",
        },
        CitySlug {
            name: "Buenos Aires",
            keyword: "highest-temperature-in-buenos-aires",
        },
        CitySlug {
            name: "Tel Aviv",
            keyword: "highest-temperature-in-tel-aviv",
        },
        CitySlug {
            name: "Istanbul",
            keyword: "highest-temperature-in-istanbul",
        },
        CitySlug {
            name: "Chongqing",
            keyword: "highest-temperature-in-chongqing",
        },
        CitySlug {
            name: "Wuhan",
            keyword: "highest-temperature-in-wuhan",
        },
        CitySlug {
            name: "San Francisco",
            keyword: "highest-temperature-in-san-francisco",
        },
        CitySlug {
            name: "Chengdu",
            keyword: "highest-temperature-in-chengdu",
        },
        CitySlug {
            name: "Denver",
            keyword: "highest-temperature-in-denver",
        },
        CitySlug {
            name: "Houston",
            keyword: "highest-temperature-in-houston",
        },
        CitySlug {
            name: "Sao Paulo",
            keyword: "highest-temperature-in-sao-paulo",
        },
        CitySlug {
            name: "Mexico City",
            keyword: "highest-temperature-in-mexico-city",
        },
        CitySlug {
            name: "Jakarta",
            keyword: "highest-temperature-in-jakarta",
        },
        CitySlug {
            name: "Amsterdam",
            keyword: "highest-temperature-in-amsterdam",
        },
        CitySlug {
            name: "Helsinki",
            keyword: "highest-temperature-in-helsinki",
        },
        CitySlug {
            name: "Kuala Lumpur",
            keyword: "highest-temperature-in-kuala-lumpur",
        },
        CitySlug {
            name: "Busan",
            keyword: "highest-temperature-in-busan",
        },
        CitySlug {
            name: "Panama City",
            keyword: "highest-temperature-in-panama-city",
        },
    ]
}
