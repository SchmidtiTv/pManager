# Pmanager
Its a Project manager which oversees all my local project. I need it due to limited storage on my PC.
When I register a Project it should check if the Project has a git repo. And it should select what type of project it is (python, C(++, #), Java/Kotlin/Android, javascript/typescript). Then after 60 days, without a change, it should strip the packages like the Node Modules. After 120 Days it should archive the folder to a 7ziped .pproject. When the project has uncommited stuff it should get hold back, and only get archive after 160 Days from the last edit.
The data is holded in a sqllite database. It contain per row a Project with the state. the Path and if acrived a path to the archived file.
The project is written in rust and a cli project, it has
- pmanager or pman - prefix
- pman list - list all the register projects
- pman project add <path> - registers a project and ask for a name for the project
- pman project cleanup - checks every projects a remove nonexisting projects from the database
- pman project remove <path> - unregisters a project
- pman run - checks all the project if the are ready for achiving or stipping. and execute it
- pman run --dry-run - checks all the project if the are ready for achiving or stipping without executing it
- pman auto-run add - Adds a auto run to the pc
- pman auto-run remove - Removes a auto run to the pc

---

When pman run is beeing executed is goes through every folder, goes through a exact workflow:
- checks if the folder a has a .pmanagerignore
- .pmanagerignore exist. it ignores the files defind in it in the last change check

--
When a project is dirty is should still get archived after the 160 days

---

When pmanager is installed it checks in his directory for a stip folder with a subfolders

pmanager.exe
strip
    - profiles.json     - Collection of a the stripping profiles
    - java.profile.json - A stipping profile

---

A profile should look like this:
```json
{
	"language": "python",
	"detection": [
		{
			"type": "file",
			"element": "pyproject.toml"
		},
		{
			"type": "folder",
			"element": ".venv"
		}
	],
	"strippable": [
		{
			"type": "folder",
			"element": ".venv"
		},
		{
			"type": "file",
			"element": "uv.lock"
		}
	]
}
```

Index file for all profiles:

```json
[
	{
		"language": "python",
		"file": "./python.profile.json"
	}
]
```
