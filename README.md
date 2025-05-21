[@HemoroidBattleBot](https://t.me/HemoroidBattleBot)
============================================

[![CI Build](https://github.com/kozalosev/DickGrowerBot/actions/workflows/ci-build.yaml/badge.svg?branch=main&event=push)](https://github.com/kozalosev/DickGrowerBot/actions/workflows/ci-build.yaml)

A parody-style competitive game bot for group chats where players try to reduce the protrusion of their virtual hemorrhoids with daily treatments. Players compete to achieve the smallest protrusion level (in centimeters) and participate in humorous "battles" with other chat members.

Additional mechanics
--------------------
_(compared with some competitors)_

* **The Hemorrhoid of the Day** daily contest to shrink a randomly chosen hemorrhoid for a bit more.
* A way to play the game without the necessity to add the bot into a group (via inline queries with a callback button).
* Import from other similar bots (not tested! help of its users is required).
* "Anal Penetration Challenge" battles with statistics.

### Soon (but not very, I guess)
* An option to show mercy and return the treatment advantage for the battle back;
* Support for those who lose battles the most;
* More perks and anti-hemorrhoid treatments;
* Achievements for consistent treatment;
* Referral promo codes;
* Global monthly events;
* Medical supply shop.

Features
--------
* True system random from the environment's chaos by usage of the `get_random()` syscall (`BCryptGenRandom` on Windows, or other alternatives on different OSes);
* English and Persian translations;
* Prometheus-like metrics;
* Health tips and hemorrhoid treatment advice.

Game Commands
------------
* `/shrink` - Apply daily treatment to your hemorrhoid (70% chance to shrink, 30% to swell)
* `/level` - Check your current hemorrhoid protrusion level
* `/top` - View the leaderboard of people with smallest hemorrhoids
* `/worst` - View those with the most severe hemorrhoid conditions
* `/penetrate` or `/buttfight` - Challenge someone to an Anal Penetration Battle
* `/clench` - Try to activate your pelvic muscles to reduce battle damage
* `/tip` - Get a random anti-hemorrhoid tip

Technical stuff
---------------

### Requirements to run
* PostgreSQL;
* _\[optional]_ Docker (it makes the configuration a lot easier);
* _\[for webhook mode]_ a frontal proxy server with TLS support ([nginx-proxy](https://github.com/nginx-proxy/nginx-proxy), for example).

### How to rebuild .sqlx queries?
_(to build the application without a running RDBMS)_

```shell
cargo sqlx prepare -- --tests
```

### Adjustment hints

You may want to change the value of the shrink/swell chance ratio in the code to adjust how frequently players experience improvement versus worsening of their condition.

### How to disable a command?

Most commands can be hidden from both lists: command hints and inline results. To do so, specify an environment variable like `DISABLE_CMD_STATS` (where `STATS` is a command key) with any value.
Don't forget to pass this variable to the container by adding it to the `docker-compose.yml` file!
