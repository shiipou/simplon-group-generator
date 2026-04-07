use rand::seq::SliceRandom;
use rusqlite::{params, Connection};
use serde_json;
use std::collections::HashMap;
use std::env;
use std::fs;

/// Parse les arguments de la ligne de commande et renvoie la taille de groupe souhaitée.
fn parse_group_size() -> usize {
    let args: Vec<String> = env::args().collect();
    let mut group_size: usize = 2;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--size" | "-s" => {
                i += 1;
                if i < args.len() {
                    group_size = args[i]
                        .parse()
                        .expect("La taille de groupe doit être un entier positif");
                } else {
                    eprintln!("Erreur : --size nécessite une valeur");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                println!("Usage: simplon-group-generator [OPTIONS]");
                println!();
                println!("Options:");
                println!("  -s, --size <N>  Nombre de personnes par groupe (défaut : 2)");
                println!("  -h, --help      Affiche cette aide");
                std::process::exit(0);
            }
            _ => {
                eprintln!("Argument inconnu : {}", args[i]);
                eprintln!("Utiliser --help pour afficher l'aide");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    if group_size < 2 {
        eprintln!("Erreur : la taille de groupe doit être au moins 2");
        std::process::exit(1);
    }

    group_size
}

/// Crée la table des groupes si elle n'existe pas encore, et migre l'ancien schéma si nécessaire.
fn init_db(conn: &Connection) {
    // Vérifier si l'ancien schéma existe (avec member_a / member_b).
    let has_old_schema: bool = conn
        .prepare("SELECT member_a FROM groups LIMIT 1")
        .is_ok();

    if has_old_schema {
        migrate_db(conn);
    } else {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS group_members (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                brief_id  INTEGER NOT NULL,
                group_id  INTEGER NOT NULL,
                member    TEXT NOT NULL
            );",
        )
        .expect("Impossible de créer la table group_members");
    }
}

/// Migre l'ancienne table `groups` (member_a, member_b) vers `group_members` (group_id, member).
fn migrate_db(conn: &Connection) {
    println!("🔄 Migration de la base de données vers le nouveau schéma…");

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS group_members (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            brief_id  INTEGER NOT NULL,
            group_id  INTEGER NOT NULL,
            member    TEXT NOT NULL
        );",
    )
    .expect("Impossible de créer la table group_members");

    // Lire toutes les anciennes paires.
    let mut stmt = conn
        .prepare("SELECT brief_id, member_a, member_b FROM groups WHERE member_a != '' AND member_b != ''")
        .expect("Requête invalide");

    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("Erreur lors de la lecture des anciens groupes")
        .filter_map(|r| r.ok())
        .collect();

    // Dans l'ancien schéma, chaque ligne est un duo distinct.
    // On leur attribue un group_id séquentiel par brief_id.
    let mut group_id_counter: HashMap<i64, i64> = HashMap::new();

    for (brief_id, member_a, member_b) in &rows {
        let gid = group_id_counter.entry(*brief_id).or_insert(0);
        *gid += 1;
        let group_id = *gid;

        conn.execute(
            "INSERT INTO group_members (brief_id, group_id, member) VALUES (?1, ?2, ?3)",
            params![brief_id, group_id, member_a],
        )
        .expect("Erreur lors de la migration (member_a)");

        conn.execute(
            "INSERT INTO group_members (brief_id, group_id, member) VALUES (?1, ?2, ?3)",
            params![brief_id, group_id, member_b],
        )
        .expect("Erreur lors de la migration (member_b)");
    }

    // Supprimer l'ancienne table.
    conn.execute_batch("DROP TABLE groups;")
        .expect("Impossible de supprimer l'ancienne table groups");

    println!("✔ Migration terminée ({} anciens duos migrés).", rows.len());
}

/// Renvoie la paire triée pour garantir l'unicité (A,B) == (B,A).
fn normalize_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Construit une matrice de comptage : combien de fois chaque duo est apparu dans le même groupe.
fn build_pair_counts(conn: &Connection) -> HashMap<(String, String), i64> {
    let mut counts: HashMap<(String, String), i64> = HashMap::new();

    let mut stmt = conn
        .prepare(
            "SELECT a.member, b.member, COUNT(DISTINCT a.brief_id) as cnt
             FROM group_members a
             JOIN group_members b ON a.brief_id = b.brief_id AND a.group_id = b.group_id
             WHERE a.member < b.member
             GROUP BY a.member, b.member",
        )
        .expect("Requête invalide");

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .expect("Erreur lors de la lecture des paires");

    for row in rows {
        let (a, b, cnt) = row.unwrap();
        let key = normalize_pair(&a, &b);
        *counts.entry(key).or_insert(0) += cnt;
    }
    counts
}

/// Renvoie le score d'un duo : le nombre de fois où ils ont déjà été ensemble.
fn pair_score(counts: &HashMap<(String, String), i64>, a: &str, b: &str) -> i64 {
    let key = normalize_pair(a, b);
    *counts.get(&key).unwrap_or(&0)
}

/// Calcule le score total d'ajout d'un candidat à un groupe existant :
/// somme des scores de paires avec chaque membre du groupe.
fn candidate_score(
    counts: &HashMap<(String, String), i64>,
    group: &[String],
    candidate: &str,
) -> i64 {
    group
        .iter()
        .map(|member| pair_score(counts, member, candidate))
        .sum()
}

/// Génère des groupes de `group_size` personnes en minimisant le score total
/// (membres les moins souvent ensemble).
///
/// Algorithme glouton avec shuffles aléatoires :
///  1. Mélanger la liste des étudiants.
///  2. Pour chaque étudiant non encore assigné, démarrer un nouveau groupe et
///     y ajouter les meilleurs candidats restants.
///  3. Répéter N fois et garder la meilleure combinaison.
///  4. Les éventuels étudiants restants (si le total n'est pas divisible par
///     group_size) forment un dernier groupe plus petit.
fn generate_groups(
    students: &[String],
    counts: &HashMap<(String, String), i64>,
    group_size: usize,
) -> Vec<Vec<String>> {
    let mut rng = rand::rng();
    let mut best_groups: Vec<Vec<String>> = Vec::new();
    let mut best_total_score = i64::MAX;

    let iterations = 10_000; // nombre de tentatives aléatoires

    for _ in 0..iterations {
        let mut pool: Vec<&str> = students.iter().map(|s| s.as_str()).collect();
        pool.shuffle(&mut rng);

        let mut groups: Vec<Vec<String>> = Vec::new();
        let mut used = vec![false; pool.len()];
        let mut total_score: i64 = 0;

        for i in 0..pool.len() {
            if used[i] {
                continue;
            }

            // Démarrer un nouveau groupe avec l'étudiant i
            used[i] = true;
            let mut group = vec![pool[i].to_string()];

            // Ajouter group_size - 1 membres supplémentaires
            while group.len() < group_size {
                let mut best_j: Option<usize> = None;
                let mut best_s = i64::MAX;

                for j in 0..pool.len() {
                    if used[j] {
                        continue;
                    }
                    let s = candidate_score(counts, &group, pool[j]);
                    if s < best_s {
                        best_s = s;
                        best_j = Some(j);
                    }
                }

                if let Some(j) = best_j {
                    used[j] = true;
                    total_score += best_s;
                    group.push(pool[j].to_string());
                } else {
                    break; // Plus d'étudiants disponibles
                }
            }

            groups.push(group);
        }

        if total_score < best_total_score {
            best_total_score = total_score;
            best_groups = groups;
        }
    }

    println!("Score total de la combinaison choisie : {best_total_score}");
    best_groups
}

/// Sauvegarde les groupes dans la DB : une ligne par membre, liée par group_id.
fn save_groups(conn: &Connection, groups: &[Vec<String>]) {
    let next_brief_id: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(brief_id), 0) + 1 FROM group_members",
            [],
            |r| r.get(0),
        )
        .unwrap_or(1);

    for (i, group) in groups.iter().enumerate() {
        let group_id = (i as i64) + 1;
        for member in group {
            conn.execute(
                "INSERT INTO group_members (brief_id, group_id, member) VALUES (?1, ?2, ?3)",
                params![next_brief_id, group_id, member],
            )
            .expect("Impossible d'enregistrer un membre");
        }
    }

    println!("✔ Groupes enregistrés avec brief_id = {next_brief_id}");
}

/// Affiche les groupes générés.
fn print_groups(groups: &[Vec<String>]) {
    println!("\n╔══════════════════════════════════════════════╗");
    println!("║          NOUVEAUX GROUPES GÉNÉRÉS            ║");
    println!("╠══════════════════════════════════════════════╣");

    for (i, group) in groups.iter().enumerate() {
        let num = i + 1;
        for (j, member) in group.iter().enumerate() {
            if j == 0 {
                println!("║ Groupe {num:>2}: {member}");
            } else {
                println!("║            {member}");
            }
        }
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
    // 1. Lire la taille de groupe souhaitée
    let group_size = parse_group_size();

    // 2. Lire les étudiants
    let data = fs::read_to_string("students.json").expect("Impossible de lire students.json");
    let students: Vec<String> =
        serde_json::from_str(&data).expect("Format invalide dans students.json");

    println!("📋 {} apprenants chargés.", students.len());
    println!("👥 Taille de groupe demandée : {group_size}");

    // 3. Ouvrir / créer la base SQLite
    let conn = Connection::open("db.sqlite").expect("Impossible d'ouvrir db.sqlite");
    init_db(&conn);

    // 4. Compter les duos existants
    let counts = build_pair_counts(&conn);
    println!("📦 {} paires distinctes en base.", counts.len());

    // 5. Générer les nouveaux groupes
    let groups = generate_groups(&students, &counts, group_size);

    // 6. Sauvegarder dans la base
    save_groups(&conn, &groups);

    // 7. Affichage
    print_groups(&groups);

    // 8. Matrice des rencontres
    print_matrix(&conn, &students);
}
