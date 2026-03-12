# simplon-group-generator
Generate a group of student from a list of students

## How to use ? 

First, you'll need to build the app : 
```bash
cargo build --release
mv ./target/release/simplon-group-generator ./
```

Just write a json array of students name in the students.json file and run the program with :
```bash
./simplon-group-generator
```

It will write output to the console and into the database db.sqlite. Don't forget to commit and push the database to never loose your previously generated groups.

Next time you'll run it, the groups won't be formed with 2 people together if they already been grouped a one time.
