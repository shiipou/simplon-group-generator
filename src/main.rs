use rand::seq::SliceRandom;
use rusqlite::{params, Connection};
use serde_json;
use std::collections::HashMap;
use std::fs;

/// Crée la table des groupes si elle n'existe pas encore.
fn init_db(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS groups (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            brief_id  INTEGER NOT NULL,
            member_a  TEXT NOT NULL,
            member_b  TEXT NOT NULL
        );",
    )
    .expect("Impossible de créer la table groups");
}

/// Renvoie la paire triée pour garantir l'unicité (A,B) == (B,A).
fn normalize_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Construit une matrice de comptage : combien de fois chaque duo est apparu.
fn build_pair_counts(conn: &Connection) -> HashMap<(String, String), i64> {
    let mut counts: HashMap<(String, String), i64> = HashMap::new();

    let mut stmt = conn
        .prepare("SELECT member_a, member_b, COUNT(*) as cnt FROM groups GROUP BY member_a, member_b")
        .expect("Requête invalide");

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .expect("Erreur lors de la lecture des duos");

    for row in rows {
        let (a, b, cnt) = row.unwrap();
        counts.insert((a, b), cnt);
    }
    counts
}

/// Renvoie le score d'un duo : le nombre de fois où ils ont déjà été ensemble.
fn pair_score(counts: &HashMap<(String, String), i64>, a: &str, b: &str) -> i64 {
    let key = normalize_pair(a, b);
    *counts.get(&key).unwrap_or(&0)
}

/// Génère des duos en minimisant le score total (duos les moins souvent ensemble).
///
/// Algorithme glouton avec shuffles aléatoires :
///  1. Mélanger la liste des étudiants.
///  2. Pour chaque étudiant non encore apparié, lui trouver le partenaire
///     restant avec le score le plus bas.
///  3. Répéter N fois et garder la meilleure combinaison.
fn generate_groups(
    students: &[String],
    counts: &HashMap<(String, String), i64>,
) -> Vec<(String, String)> {
    let mut rng = rand::rng();
    let mut best_groups: Vec<(String, String)> = Vec::new();
    let mut best_total_score = i64::MAX;

    let iterations = 10_000; // nombre de tentatives aléatoires

    for _ in 0..iterations {
        let mut pool: Vec<&str> = students.iter().map(|s| s.as_str()).collect();
        pool.shuffle(&mut rng);

        let mut groups: Vec<(String, String)> = Vec::new();
        let mut used = vec![false; pool.len()];
        let mut total_score: i64 = 0;

        for i in 0..pool.len() {
            if used[i] {
                continue;
            }

            let mut best_j: Option<usize> = None;
            let mut best_s = i64::MAX;

            for j in (i + 1)..pool.len() {
                if used[j] {
                    continue;
                }
                let s = pair_score(counts, pool[i], pool[j]);
                if s < best_s {
                    best_s = s;
                    best_j = Some(j);
                }
            }

            if let Some(j) = best_j {
                used[i] = true;
                used[j] = true;
                total_score += best_s;
                groups.push((pool[i].to_string(), pool[j].to_string()));
            }
            // Si nombre impair, le dernier reste seul (géré plus bas).
        }

        // Gérer un étudiant restant (nombre impair).
        for (i, &is_used) in used.iter().enumerate() {
            if !is_used {
                // Étudiant restant (nombre impair) — on le marque avec un membre vide.
                // Il sera rattaché au dernier groupe pour former un trio à l'affichage.
                groups.push((pool[i].to_string(), String::new()));
                break;
            }
        }

        if total_score < best_total_score {
            best_total_score = total_score;
            best_groups = groups;
        }
    }

    println!("Score total de la combinaison choisie : {best_total_score}");
    best_groups
}

/// Sauvegarde les nouveaux duos dans la DB.
fn save_groups(conn: &Connection, groups: &[(String, String)]) {
    let next_brief_id: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(brief_id), 0) + 1 FROM groups",
            [],
            |r| r.get(0),
        )
        .unwrap_or(1);

    for (a, b) in groups {
        if b.is_empty() {
            continue; // étudiant solitaire (nombre impair), pas un vrai duo
        }
        let (na, nb) = normalize_pair(a, b);
        conn.execute(
            "INSERT INTO groups (brief_id, member_a, member_b) VALUES (?1, ?2, ?3)",
            params![next_brief_id, na, nb],
        )
        .expect("Impossible d'enregistrer un groupe");
    }

    println!("✔ Groupes enregistrés avec brief_id = {next_brief_id}");
}

/// Affiche les groupes générés.
fn print_groups(groups: &[(String, String)], students: &[String]) {
    println!("\n╔══════════════════════════════════════════════╗");
    println!("║          NOUVEAUX GROUPES GÉNÉRÉS            ║");
    println!("╠══════════════════════════════════════════════╣");

    // Trouver l'éventuel solitaire (nombre impair).
    let solo: Option<&str> = groups
        .iter()
        .find(|(_, b)| b.is_empty())
        .map(|(a, _)| a.as_str());

    let real_groups: Vec<&(String, String)> = groups.iter().filter(|(_, b)| !b.is_empty()).collect();

    for (i, (a, b)) in real_groups.iter().enumerate() {
        let num = i + 1;
        // Si c'est le dernier groupe et qu'il y a un solitaire, on forme un trio.
        if let Some(extra) = solo {
            if i == real_groups.len() - 1 {
                println!("║ Groupe {num:>2}: {a}");
                println!("║            {b}");
                println!("║            {extra}");
                continue;
            }
        }
        println!("║ Groupe {num:>2}: {a}");
        println!("║            {b}");
    }

    if solo.is_none() && students.len() % 2 == 0 {
        // Tous en duos, rien de spécial.
    }

    println!("╚══════════════════════════════════════════════╝");
}

/// Affiche la matrice des rencontres (après enregistrement).
fn print_matrix(conn: &Connection, students: &[String]) {
    let counts = build_pair_counts(conn);

    println!("\n📊 Matrice des rencontres :");

    // Créer des labels courts (prénom seulement).
    let labels: Vec<&str> = students
        .iter()
        .map(|s| s.split_whitespace().last().unwrap_or(s.as_str()))
        .collect();

    // Largeur de la première colonne
    let max_label = labels.iter().map(|l| l.len()).max().unwrap_or(10);

    // En-tête
    print!("{:>width$} │", "", width = max_label);
    for l in &labels {
        // Prendre les 3 premiers caractères (safe UTF-8)
        let short: String = l.chars().take(3).collect();
        print!(" {:>3}", short);
    }
    println!();
    println!(
        "{:─>width$}─┼{}",
        "",
        "────".repeat(labels.len()),
        width = max_label
    );

    for (i, si) in students.iter().enumerate() {
        print!("{:>width$} │", labels[i], width = max_label);
        for (j, sj) in students.iter().enumerate() {
            if i == j {
                print!("   .");
            } else {
                let score = pair_score(&counts, si, sj);
                if score == 0 {
                    print!("   -");
                } else {
                    print!(" {:>3}", score);
                }
            }
        }
        println!();
    }
}

fn main() {
    // 1. Lire les étudiants
    let data = fs::read_to_string("students.json").expect("Impossible de lire students.json");
    let students: Vec<String> =
        serde_json::from_str(&data).expect("Format invalide dans students.json");

    println!("📋 {} apprenants chargés.", students.len());

    // 2. Ouvrir / créer la base SQLite
    let conn = Connection::open("db.sqlite").expect("Impossible d'ouvrir db.sqlite");
    init_db(&conn);

    // 4. Compter les duos existants
    let counts = build_pair_counts(&conn);
    println!("📦 {} duos distincts en base.", counts.len());

    // 5. Générer les nouveaux groupes
    let groups = generate_groups(&students, &counts);

    // 6. Sauvegarder dans la base
    save_groups(&conn, &groups);

    // 7. Affichage
    print_groups(&groups, &students);

    // 8. Matrice des rencontres
    print_matrix(&conn, &students);
}
