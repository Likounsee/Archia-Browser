use std::{
    io::{self, Write},
    path::PathBuf,
};

use archia_browser::{
    html::Node,
    net::{
        pipeline::{NetworkPipeline, PolicyDecision, RequestPolicyEngine, ResourceKind},
        DocumentLoader, HttpTransport, Request, Url,
    },
    theme::ThemeManager,
    Browser,
};

const VIEWPORT_WIDTH: u32 = 1100;
const VIEWPORT_HEIGHT: u32 = 760;
const MAX_DISPLAYED_LINKS: usize = 100;

struct BrowserPolicy;

impl RequestPolicyEngine for BrowserPolicy {
    fn decide(&self, request: &Request) -> PolicyDecision {
        match request.url.scheme() {
            "http" | "https" => PolicyDecision::Allow,
            _ => PolicyDecision::Block,
        }
    }
}

fn viewport() -> archia_browser::layout::LayoutViewport {
    archia_browser::layout::LayoutViewport::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
}

#[cfg(windows)]
fn theme_directory() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ArchiaBrowser")
        .join("themes")
}

#[cfg(not(windows))]
fn theme_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("archia-browser")
        .join("themes")
}

fn print_themes(themes: &ThemeManager) {
    println!("Themes disponibles :");
    for theme in themes.themes() {
        let active = if theme.manifest.id == themes.active_id() {
            " [actif]"
        } else {
            ""
        };
        println!(
            "  {} — {}{}",
            theme.manifest.id, theme.manifest.name, active
        );
    }
    println!("Changer de theme : :theme ID");
}

fn parse_address(input: &str) -> Result<Url, archia_browser::net::UrlError> {
    let input = input.trim();
    let address = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    Url::parse(&address)
}

fn collect_links(document: &Node) -> Vec<(String, String)> {
    document
        .find_elements_by_tag("a")
        .into_iter()
        .filter_map(|node| {
            let href = node.link_href()?.trim();
            if href.is_empty() {
                return None;
            }
            let label = node.text_content();
            let label = if label.trim().is_empty() {
                href.to_owned()
            } else {
                label.split_whitespace().collect::<Vec<_>>().join(" ")
            };
            Some((label, href.to_owned()))
        })
        .take(MAX_DISPLAYED_LINKS)
        .collect()
}

fn print_page(browser: &Browser) {
    let Some(page) = browser.current_page() else {
        println!("\nAucune page chargee. Entrez une adresse HTTP ou HTTPS.\n");
        return;
    };

    let title = page.title().unwrap_or_else(|| "Archia Browser".to_owned());
    println!("\n{title}");
    println!("{}", "=".repeat(title.chars().count().min(72)));
    println!(
        "URL: {}\n",
        page.url().map(ToString::to_string).unwrap_or_default()
    );

    let text = page.document.visible_text_content();
    let mut line_length = 0usize;
    for word in text.split_whitespace() {
        let word_length = word.chars().count();
        if line_length > 0 && line_length + word_length + 1 > 78 {
            println!();
            line_length = 0;
        }
        if line_length > 0 {
            print!(" ");
            line_length += 1;
        }
        print!("{word}");
        line_length += word_length;
    }
    println!("\n");

    let links = collect_links(&page.document);
    if !links.is_empty() {
        println!(
            "{} lien(s) detecte(s). Tapez :links pour les afficher.\n",
            links.len()
        );
    }
}

fn load_url(
    browser: &mut Browser,
    loader: &DocumentLoader<BrowserPolicy, HttpTransport>,
    url: Url,
) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        eprintln!("Seules les adresses HTTP et HTTPS sont acceptees.");
        return false;
    }

    let mut request = Request::new(url);
    // Send browser-like navigation metadata. In particular, Wikipedia and
    // other public sites reject requests that omit a meaningful User-Agent.
    request.headers.insert(
        "accept".into(),
        "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8".into(),
    );
    request
        .headers
        .insert("accept-language".into(), "fr-FR,fr;q=0.9,en;q=0.8".into());
    request.headers.insert(
        "user-agent".into(),
        format!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) ArchiaBrowser/{}",
            browser.version()
        ),
    );
    request.policy.resource_kind = ResourceKind::Document;
    request.policy.top_level_navigation = true;

    print!("Chargement de {} ... ", request.url);
    let _ = io::stdout().flush();
    match browser.load_request(loader, &request, viewport()) {
        Ok(_) => {
            println!("OK");
            print_page(browser);
            true
        }
        Err(error) => {
            println!("ECHEC");
            eprintln!("Impossible de charger la page: {error:?}");
            false
        }
    }
}

fn load_address(
    browser: &mut Browser,
    loader: &DocumentLoader<BrowserPolicy, HttpTransport>,
    input: &str,
) -> bool {
    match parse_address(input) {
        Ok(url) => load_url(browser, loader, url),
        Err(error) => {
            eprintln!("Adresse invalide: {error:?}");
            false
        }
    }
}

fn print_links(browser: &Browser) {
    let Some(page) = browser.current_page() else {
        println!("Aucune page chargee.");
        return;
    };
    let links = collect_links(&page.document);
    if links.is_empty() {
        println!("Aucun lien exploitable sur cette page.");
        return;
    }
    println!("Liens de la page :");
    for (index, (label, href)) in links.iter().enumerate() {
        println!("  {}. {}  ({})", index + 1, label, href);
    }
    println!("Pour ouvrir un lien : :open NUMERO");
}

fn open_link(
    browser: &mut Browser,
    loader: &DocumentLoader<BrowserPolicy, HttpTransport>,
    index: usize,
) -> bool {
    let Some(page) = browser.current_page() else {
        eprintln!("Aucune page chargee.");
        return false;
    };
    let links = collect_links(&page.document);
    let Some((_, href)) = links.get(index) else {
        eprintln!("Numero de lien invalide. Utilisez :links pour voir la liste.");
        return false;
    };
    let Some(base) = page.url() else {
        eprintln!("La page courante ne possede pas d'URL de base.");
        return false;
    };
    let url = match base.resolve(href) {
        Ok(url) => url,
        Err(error) => {
            eprintln!("Lien invalide: {error:?}");
            return false;
        }
    };
    load_url(browser, loader, url)
}

fn clear_terminal() {
    // ANSI clear-screen + cursor-home works in modern Windows Terminal,
    // PowerShell, and the usual Unix terminals without spawning a shell.
    print!("\\x1B[2J\\x1B[H");
    let _ = io::stdout().flush();
}

fn print_help() {
    println!("Commandes Archia Browser :");
    println!("  URL             Naviguer vers une adresse (HTTPS par defaut)");
    println!("  :links          Afficher les liens de la page");
    println!("  :open NUMERO    Ouvrir un lien affiche par :links");
    println!("  :back           Page precedente");
    println!("  :forward        Page suivante");
    println!("  :reload         Recharger la page");
    println!("  :url            Afficher l'adresse courante");
    println!("  :title          Afficher le titre de la page");
    println!("  :themes         Lister les themes disponibles");
    println!("  :theme ID       Selectionner un theme");
    println!("  :theme install PATH  Installer un manifeste de theme");
    println!("  :theme remove ID     Desinstaller un theme personnalise");
    println!("  :clear / clear  Effacer l'ecran du terminal");
    println!("  :help           Afficher cette aide");
    println!("  :quit           Quitter Archia Browser");
}

fn main() {
    let mut browser = Browser::new();
    let pipeline = NetworkPipeline::new(BrowserPolicy);
    let loader = browser.document_loader(pipeline, HttpTransport::new());
    let mut theme_manager = match ThemeManager::load(theme_directory()) {
        Ok(manager) => Some(manager),
        Err(error) => {
            eprintln!("Gestion des themes indisponible: {error}");
            None
        }
    };

    println!("Archia Browser {}", browser.version());
    println!("Navigateur experimental Windows/terminal — moteur Archia natif.");
    if let Some(themes) = theme_manager.as_ref() {
        println!("Theme actif : {}", themes.active_theme().manifest.name);
    }
    println!("Tapez :help pour les commandes.\n");

    if let Some(address) = std::env::args().nth(1) {
        load_address(&mut browser, &loader, &address);
    }

    let stdin = io::stdin();
    let mut input = String::new();
    loop {
        print!("archia> ");
        if io::stdout().flush().is_err() {
            break;
        }
        input.clear();
        match stdin.read_line(&mut input) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let command = input.trim();
        if command.is_empty() {
            continue;
        }

        if let Some(number) = command.strip_prefix(":open ") {
            match number.trim().parse::<usize>() {
                Ok(number) if number > 0 => {
                    open_link(&mut browser, &loader, number - 1);
                }
                _ => eprintln!("Syntaxe : :open NUMERO"),
            }
            continue;
        }

        if command == ":themes" {
            if let Some(themes) = theme_manager.as_ref() {
                print_themes(themes);
            } else {
                eprintln!("Gestion des themes indisponible.");
            }
            continue;
        }
        if command == ":theme" {
            if let Some(themes) = theme_manager.as_ref() {
                println!(
                    "Theme actif : {} ({})",
                    themes.active_theme().manifest.name,
                    themes.active_id()
                );
            } else {
                eprintln!("Gestion des themes indisponible.");
            }
            continue;
        }
        if let Some(path) = command.strip_prefix(":theme install ") {
            let result = match (theme_manager.as_mut(), std::fs::read_to_string(path.trim())) {
                (Some(themes), Ok(manifest)) => themes.install_manifest(&manifest),
                (Some(_), Err(error)) => {
                    eprintln!("Lecture du manifeste impossible: {error}");
                    continue;
                }
                (None, _) => {
                    eprintln!("Gestion des themes indisponible.");
                    continue;
                }
            };
            match result {
                Ok(()) => println!("Theme installe."),
                Err(error) => eprintln!("Installation impossible: {error}"),
            }
            continue;
        }
        if let Some(id) = command.strip_prefix(":theme remove ") {
            match theme_manager.as_mut() {
                Some(themes) => match themes.uninstall(id.trim()) {
                    Ok(()) => println!("Theme desinstalle."),
                    Err(error) => eprintln!("Desinstallation impossible: {error}"),
                },
                None => eprintln!("Gestion des themes indisponible."),
            }
            continue;
        }

        if let Some(id) = command.strip_prefix(":theme ") {
            match theme_manager.as_mut() {
                Some(themes) => match themes.select(id.trim()) {
                    Ok(()) => println!(
                        "Theme selectionne : {}",
                        themes.active_theme().manifest.name
                    ),
                    Err(error) => eprintln!("Selection du theme impossible: {error}"),
                },
                None => eprintln!("Gestion des themes indisponible."),
            }
            continue;
        }

        match command {
            ":quit" | ":q" | "quit" | "exit" => break,
            ":clear" | "clear" => clear_terminal(),
            ":help" | ":h" => print_help(),
            ":links" => print_links(&browser),
            ":url" => match browser.current_url() {
                Some(url) => println!("{url}"),
                None => println!("Aucune adresse courante."),
            },
            ":title" => match browser.current_page().and_then(|page| page.title()) {
                Some(title) => println!("{title}"),
                None => println!("Aucun titre disponible."),
            },
            ":back" => match browser.go_back(&loader, viewport()) {
                Ok(_) => print_page(&browser),
                Err(error) => eprintln!("Retour impossible: {error:?}"),
            },
            ":forward" => match browser.go_forward(&loader, viewport()) {
                Ok(_) => print_page(&browser),
                Err(error) => eprintln!("Avance impossible: {error:?}"),
            },
            ":reload" => match browser.reload(&loader, viewport()) {
                Ok(_) => print_page(&browser),
                Err(error) => eprintln!("Rechargement impossible: {error:?}"),
            },
            _ => {
                load_address(&mut browser, &loader, command);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_without_scheme_defaults_to_https() {
        assert_eq!(
            parse_address("example.org").unwrap().to_string(),
            "https://example.org/"
        );
    }

    #[test]
    fn explicit_http_scheme_is_preserved() {
        assert_eq!(
            parse_address("http://example.org/path")
                .unwrap()
                .to_string(),
            "http://example.org/path"
        );
    }

    #[test]
    fn address_is_trimmed_before_parsing() {
        assert_eq!(
            parse_address("  https://example.org/path  ")
                .unwrap()
                .to_string(),
            "https://example.org/path"
        );
    }

    #[test]
    fn unsupported_schemes_are_rejected_by_navigation() {
        let url = parse_address("file:///C:/secret.txt").unwrap();
        assert!(!matches!(url.scheme(), "http" | "https"));
    }

    #[test]
    fn links_are_collected_in_document_order_and_ignore_empty_hrefs() {
        let mut document = Node::document();
        let mut first = Node::element("a");
        first.set_attribute("href", "/first");
        first.append(Node::text("First link"));
        document.append(first);

        let mut empty = Node::element("a");
        empty.set_attribute("href", "  ");
        empty.append(Node::text("Ignored"));
        document.append(empty);

        let mut second = Node::element("a");
        second.set_attribute("href", "https://example.org/second");
        second.append(Node::text("Second"));
        document.append(second);

        assert_eq!(
            collect_links(&document),
            vec![
                ("First link".to_owned(), "/first".to_owned()),
                ("Second".to_owned(), "https://example.org/second".to_owned())
            ]
        );
    }

    #[test]
    fn link_without_text_uses_its_destination_as_label() {
        let mut document = Node::document();
        let mut anchor = Node::element("a");
        anchor.set_attribute("href", "/destination");
        document.append(anchor);

        assert_eq!(
            collect_links(&document),
            vec![("/destination".to_owned(), "/destination".to_owned())]
        );
    }
}
