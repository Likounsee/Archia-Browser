use std::io::{self, Write};

use archia_browser::{
    net::{
        pipeline::{NetworkPipeline, PolicyDecision, RequestPolicyEngine, ResourceKind},
        DocumentLoader, HttpTransport, Request, Url,
    },
    Browser,
};

struct BrowserPolicy;

impl RequestPolicyEngine for BrowserPolicy {
    fn decide(&self, request: &Request) -> PolicyDecision {
        match request.url.scheme() {
            "http" | "https" => PolicyDecision::Allow,
            _ => PolicyDecision::Block,
        }
    }
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

fn print_page(browser: &Browser) {
    let Some(page) = browser.current_page() else {
        println!("\nAucune page chargee. Entrez une adresse HTTP ou HTTPS.\n");
        return;
    };

    let title = page.title().unwrap_or_else(|| "Archia Browser".to_owned());
    println!("\n{title}");
    println!("{}", "=".repeat(title.chars().count().min(72)));
    println!("URL: {}\n", page.url().map(ToString::to_string).unwrap_or_default());

    let text = page.document.text_content();
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
}

fn load_address(
    browser: &mut Browser,
    loader: &DocumentLoader<BrowserPolicy, HttpTransport>,
    input: &str,
) -> bool {
    let url = match parse_address(input) {
        Ok(url) => url,
        Err(error) => {
            eprintln!("Adresse invalide: {error:?}");
            return false;
        }
    };
    if !matches!(url.scheme(), "http" | "https") {
        eprintln!("Seules les adresses HTTP et HTTPS sont acceptees.");
        return false;
    }

    let mut request = Request::new(url);
    request.policy.resource_kind = ResourceKind::Document;
    request.policy.top_level_navigation = true;

    print!("Chargement de {} ... ", request.url);
    let _ = io::stdout().flush();
    match browser.load_request(
        loader,
        &request,
        archia_browser::layout::LayoutViewport::new(1000, 720),
    ) {
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

fn main() {
    let mut browser = Browser::new();
    let pipeline = NetworkPipeline::new(BrowserPolicy);
    let loader = browser.document_loader(pipeline, HttpTransport::new());

    println!("Archia Browser {}", browser.version());
    println!("Navigateur experimental en mode texte — moteur Archia natif.");
    println!("Commandes: adresse | :back | :forward | :reload | :help | :quit\n");

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
        match command {
            ":quit" | ":q" | "quit" | "exit" => break,
            ":help" | ":h" => {
                println!("Entrez une URL pour naviguer.");
                println!(":back      Page precedente");
                println!(":forward   Page suivante");
                println!(":reload    Recharger la page");
                println!(":quit      Quitter Archia Browser");
            }
            ":back" => match browser.go_back(
                &loader,
                archia_browser::layout::LayoutViewport::new(1000, 720),
            ) {
                Ok(_) => print_page(&browser),
                Err(error) => eprintln!("Retour impossible: {error:?}"),
            },
            ":forward" => match browser.go_forward(
                &loader,
                archia_browser::layout::LayoutViewport::new(1000, 720),
            ) {
                Ok(_) => print_page(&browser),
                Err(error) => eprintln!("Avance impossible: {error:?}"),
            },
            ":reload" => match browser.reload(
                &loader,
                archia_browser::layout::LayoutViewport::new(1000, 720),
            ) {
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
            "https://example.org"
        );
    }

    #[test]
    fn explicit_http_scheme_is_preserved() {
        assert_eq!(
            parse_address("http://example.org/path").unwrap().to_string(),
            "http://example.org/path"
        );
    }
}
