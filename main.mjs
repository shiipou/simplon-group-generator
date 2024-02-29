import fs from 'fs'

const FILE_PATH = './last_brief.json'
const last_brief = fs.existsSync(FILE_PATH) ? JSON.parse(fs.readFileSync(FILE_PATH)) : null

let peoples = JSON.parse(fs.readFileSync('./students.json'))

/**
 * 
 * @param {string[]} group 
 * @param {string[][] | null} last_brief
 */
function hasSameGroup(group, last_brief) {
    if(last_brief == null){
        return false
    }
    return Object.values(last_brief)
        .some(
            (oneGroup)=>group
                .every(
                    (people)=>oneGroup.includes(people)
                )
        )
}

let groups = []

do{
    while(peoples.length > 0) {
        const leader = peoples.pop()
        const index = Math.floor(Math.random() * peoples.length) - 1
        const [member] = peoples.splice(index, 1)
        groups = [...groups, [leader, member]]
    }
} while(hasSameGroup(groups, last_brief))
    
fs.writeFileSync(FILE_PATH, JSON.stringify(groups))
console.log("Liste des groupes :", groups)