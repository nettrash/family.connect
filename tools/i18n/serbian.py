# -*- coding: utf-8 -*-
"""Serbian (Cyrillic) translations for family.connect, plus the mechanical
transliteration to Serbian Latin.

Cyrillic is the source of truth; Latin is DERIVED, so the two can never drift.
Register is informal (ти), matching the decision taken for the other six
languages — this is a family chat, not a bank.
"""

# Serbian Cyrillic. Latin comes from TRANSLIT below.
SR = {
# Birthdays, and the assistant's two family settings.
"Birthday": "Рођендан",
"Birthday…": "Рођендан…",
"Birthday for %@": "Рођендан за %@",
"Add Birthday…": "Додај рођендан…",
"Change Birthday…": "Промени рођендан…",
"Remove Birthday": "Уклони рођендан",
"Month": "Месец",
"Day": "Дан",
"A day and a month, with no year — so being wished a happy birthday never means publishing your age.":
  "Дан и месец, без године — честитке за рођендан никада не откривају твоје године.",
"A day and a month, with no year. Everyone in the family sees it.":
  "Дан и месец, без године. Виде га сви у породици.",
"That date doesn't exist.": "Тај датум не постоји.",
"Couldn't save that birthday. Try again.": "Рођендан није сачуван. Покушај поново.",
"Only the family owner can do that.": "То може да уради само власник породице.",
"Assistant language": "Језик асистента",
"Answers in": "Језик одговора",
"Not set": "Није подешено",
"Sees recent history": "Види недавну историју",
"The language %@ answers in when it is asked in the family chat. It is not this app's language — that follows the device. With none chosen, it answers in the language of whoever asked.":
  "Језик на ком %@ одговара када га питаш у породичном разговору. То није језик ове "
  "апликације — он прати уређај. Ако ништа није изабрано, одговор стиже на језику питања.",
"With this on, mentioning %@ in the family chat sends the last month of that chat to the assistant, so it can answer questions about what was said earlier. With it off, only the message that mentions it is sent.":
  "Када је укључено, помињање %@ у породичном разговору шаље асистенту преписку из "
  "последњих месец дана, па може да одговара и на питања о раније реченом. Када је "
  "искључено, шаље се само порука у којој се помиње.",
"Only the family owner can change this.": "Ово може да промени само власник породице.",
"Couldn't save that. Try again.": "Није сачувано. Покушај поново.",
"New message": "Нова порука",

"%@ has been signed out everywhere.": "Све сесије за %@ су затворене.",
"%@ will be signed out on every device and will need this password to sign back in. Tell it to them somewhere safe — the server has no way to email it.":
  "Све сесије за %@ биће затворене, а за поновну пријаву требаће ова лозинка. Пренеси је на сигуран начин — сервер не може да шаље мејлове.",
"which replied to %@: %@": "које је одговарало %1$@: %2$@",
"More reactions…": "Још реакција…",
"Click to remove": "Кликни да уклониш",
"Record Audio": "Сними аудио",
"Stop": "Заустави",
"Attach a File…": "Приложи датотеку…",
"That recording was too short.": "Снимак је прекратак.",
"Audio": "Аудио",
"Play": "Пусти",
"Pause": "Пауза",
"Audio, %@": "Аудио, %@",
"Statistics": "Статистика",
"Statistics…": "Статистика…",
"The family": "Породица",
"Board notes": "Белешке на табли",
"Attachments": "Прилози",
"Photos": "Фотографије",
"Videos": "Видео снимци",
"Files": "Датотеке",
"On disk": "На диску",
"Who sends what": "Ко шта шаље",
"Words only": "Само текст",
"Questions": "Питања",
"Tokens": "Токени",
"Assistant": "Асистент",
"Couldn't load statistics": "Статистика није учитана",
"Check your connection and try again.": "Провери везу и покушај поново.",
"%lld attachments, %@": "прилога: %1$lld, %2$@",
"%lld questions to the assistant": "питања асистенту: %lld",
"%@ saved by storing one copy of identical files.": "%@ уштеђено — исте датотеке се чувају у једном примерку.",
"Camera": "Камера",
"Add a message, or send it on its own.": "Додај поруку или пошаљи овако.",
"Remove attachment": "Уклони прилог",
"Activities": "Активности",
"Add a note": "Додај белешку",
"Add a note — everyone in the family sees it.": "Додај белешку — виде је сви у породици.",
"Add Note": "Додај белешку",
"Add Photo": "Додај фотографију",
"Address": "Адреса",
"Almost There": "Само још мало",
"An owner can only leave once everyone else has left (the family is then deleted). Remove the other members first, or keep the family going.":
  "Власник може да оде тек кад оду сви остали (тада се породица брише). Прво уклони друге чланове — или задржи породицу.",
"Animals & Nature": "Животиње и природа",
"Any family member can read the code to you; the owner finds it in Settings.":
  "Код ти може прочитати било ко из породице; власник га налази у подешавањима.",
"Approve": "Прихвати",
"Ask the family member who runs the server for its address.": "Питај за адресу оног ко држи сервер.",
"Attach a photo, video or file": "Приложи фотографију, видео или датотеку",
"Attachment unavailable": "Прилог није доступан",
"Board": "Табла",
"Can't reach the family server": "Породични сервер није доступан",
"Cancel": "Откажи",
"Cancel editing": "Откажи измену",
"Cancel reply": "Откажи одговор",
"Change Password": "Промена лозинке",
"Change Password…": "Промени лозинку…",
"Change Photo": "Промени фотографију",
"Change server…": "Промени сервер…",
"Chats": "Разговори",
"Checking…": "Проверавам…",
"Close": "Затвори",
"Code": "Код",
"Colour": "Боја",
"Confirm New Password": "Потврди нову лозинку",
"Connect": "Повежи се",
"Connected to %@": "Повезано са %@",
"Connecting…": "Повезивање…",
"Conversation unavailable": "Разговор није доступан",
"Copy": "Копирај",
"Couldn't open the chat": "Разговор не може да се отвори",
"Couldn't open the message store.": "Складиште порука не може да се отвори.",
"Create a family": "Направи породицу",
"Create a Family": "Направи породицу",
"Create Account": "Направи налог",
"Create Family": "Направи породицу",
"Creating account…": "Правим налог…",
"Creating…": "Правим…",
"Current Password": "Тренутна лозинка",
"Decline": "Одбиј",
"Delete": "Обриши",
"Delete Note": "Обриши белешку",
"Delete this note?": "Обрисати ову белешку?",
"Direct chats become available once another member joins the family.":
  "Лични разговори постају доступни кад се породици придружи још неко.",
"Dismiss": "Одбаци",
"Display name": "Име за приказ",
"Done": "Готово",
"Edit": "Измени",
"Edit…": "Измени…",
"edited": "измењено",
"Editing message": "Измена поруке",
"Enter the invite code a family member shared with you.": "Унеси позивни код који ти је неко из породице дао.",
"Failed. Tap to retry.": "Није успело. Додирни да покушаш поново.",
"Family": "Породица",
"Family Members": "Чланови породице",
"Family name": "Име породице",
"File": "Датотека",
"Food & Drink": "Храна и пиће",
"Gestures": "Гестови",
"Hearts": "Срца",
"Hi, %@": "Здраво, %@",
"Invite code": "Позивни код",
"Join": "Придружи се",
"Join a family": "Придружи се породици",
"Join a Family": "Придруживање породици",
"Join immediately": "Придружују се одмах",
"Join policy": "Правило приступања",
"Join requests": "Захтеви за приступ",
"Join the Family": "Придружи се породици",
"Joining…": "Придружујем се…",
"Leave Family": "Напусти породицу",
"Leave the family?": "Напустити породицу?",
"Link Previews": "Прегледи линкова",
"Local messages are removed from this device.": "Локалне поруке се бришу са овог уређаја.",
"Local messages are removed from this Mac.": "Локалне поруке се бришу са овог Mac рачунара.",
"Log In": "Пријави се",
"Log out": "Одјави се",
"Log Out": "Одјави се",
"Log out?": "Одјавити се?",
"Logging in…": "Пријављивање…",
"Manage Family": "Управљање породицом",
"Members": "Чланови",
"Members, invites and direct chats": "Чланови, позивнице и лични разговори",
"Message": "Порука",
"Messages": "Поруке",
"Messages stay on the family server; this device forgets its session.":
  "Поруке остају на породичном серверу; овај уређај заборавља своју сесију.",
"Mode": "Режим",
"More reactions": "Још реакција",
"Name": "Име",
"Need approval": "Треба одобрење",
"New Chat": "Нови разговор",
"New members": "Нови чланови",
"New Note": "Нова белешка",
"New password for %@": "Нова лозинка за %@",
"No chats yet": "Још нема разговора",
"No conversation selected": "Није изабран ниједан разговор",
"No messages yet": "Још нема порука",
"No one else yet": "Још нема никог другог",
"No pending requests": "Нема захтева на чекању",
"Note": "Белешка",
"Note from %@: %@": "Белешка од %@: %@",
"Objects & Symbols": "Предмети и симболи",
"OK": "У реду",
"Open in New Window": "Отвори у новом прозору",
"Opens the link": "Отвара линк",
"Owner": "Власник",
"Password": "Лозинка",
"Password changed": "Лозинка је промењена",
"Password reset": "Лозинка је ресетована",
"Photo": "Фотографија",
"Photo or Video": "Фотографија или видео",
"Pick a chat from the sidebar.": "Изабери разговор са бочне траке.",
"Preparing…": "Припремам…",
"Privacy": "Приватност",
"Profile": "Профил",
"Pull down to sync with the family server.": "Повуци надоле да синхронизујеш са породичним сервером.",
"React": "Реагуј",
"Reacts with %@": "Реагује са %@",
"Read": "Прочитано",
"Refresh": "Освежи",
"Reinstall Family Connect to start fresh. Your messages are safe on the family server and will re-download.":
  "Поново инсталирај Family Connect да почнеш испочетка. Твоје поруке су сигурне на породичном серверу и биће поново преузете.",
"Remove": "Уклони",
"Remove from Family": "Уклони из породице",
"Remove Photo": "Уклони фотографију",
"Remove reaction": "Уклони реакцију",
"Reply": "Одговори",
"Replying to %@": "Одговор за %@",
"Replying to %@: %@": "Одговор за %@: %@",
"Reset": "Ресетуј",
"Reset Password": "Ресетовање лозинке",
"Reset Password…": "Ресетуј лозинку…",
"Retry": "Покушај поново",
"Rotate": "Промени",
"Rotate Code": "Промени код",
"Rotate the invite code?": "Променити позивни код?",
"Rotating invalidates the current code immediately.": "Тренутни код одмах престаје да важи.",
"Save": "Сачувај",
"Save a copy": "Сачувај копију",
"Save…": "Сачувај…",
"Say something to get started.": "Напиши нешто да започнеш.",
"See who reacted": "Види ко је реаговао",
"Send": "Пошаљи",
"Sending": "Шаље се",
"Sending…": "Шаљем…",
"Sent": "Послато",
"Server": "Сервер",
"Server address": "Адреса сервера",
"Settings": "Подешавања",
"Share": "Подели",
"Share Invite": "Подели позивницу",
"Share…": "Подели…",
"Shows a preview under links in messages. Building one asks the linked website for its title and image, so that site sees a request from this device.":
  "Приказује преглед испод линкова у порукама. Да би се направио, од сајта се траже наслов и слика — тако тај сајт види захтев са овог уређаја.",
"Shows who reacted": "Приказује ко је реаговао",
"Smileys": "Смајлији",
"Someone": "Неко",
"Start fresh — you'll be the owner and can invite everyone else.":
  "Почни испочетка — бићеш власник и можеш да позовеш све остале.",
"Tap to remove": "Додирни да уклониш",
"Tap to retry": "Додирни да покушаш поново",
"The board is empty": "Табла је празна",
"The current code stops working immediately. Pending requests survive.":
  "Тренутни код одмах престаје да важи. Захтеви на чекању остају.",
"The family board": "Породична табла",
"The family owner needs to approve your request. This screen updates automatically — or pull down to check right now.":
  "Власник породице треба да одобри твој захтев. Овај екран се сам освежава — или повуци надоле да провериш одмах.",
"This names your family chat too. 1–64 characters.": "Ово је уједно и име породичног разговора. 1–64 знака.",
"Today": "Данас",
"Travel & Places": "Путовања и места",
"Try Again": "Покушај поново",
"Username": "Корисничко име",
"Usernames are 3–32 letters, digits, dots or underscores. Passwords need at least 8 characters.":
  "Корисничко име има 3–32 слова, цифре, тачке или доње црте. Лозинка мора имати бар 8 знакова.",
"Video": "Видео",
"Waiting for approval": "Чека се одобрење",
"Welcome Back": "Здраво поново",
"With approval, join requests wait here until you approve them.":
  "Уз одобрење, захтеви чекају овде док их не одобриш.",
"Written by %@": "Аутор: %@",
"Written by someone else": "Аутор је неко други",
"Yesterday": "Јуче",
"You": "Ти",
"You are the owner.": "Ти си власник.",
"You'll lose access to the family chat and your direct chats. Your history returns if you rejoin.":
  "Изгубићеш приступ породичном разговору и својим личним разговорима. Историја се враћа ако се поново придружиш.",
"You're the owner": "Ти си власник",
"Your Family": "Твоја породица",
"Your note: %@": "Твоја белешка: %@",
"Your other devices have been signed out.": "Остали твоји уређаји су одјављени.",
"Your other devices will be signed out. This one stays signed in.":
  "Остали твоји уређаји биће одјављени. Овај остаје пријављен.",
"Your request to join was declined. You can ask for a new invite code and try again.":
  "Твој захтев за приступ је одбијен. Можеш да затражиш нови позивни код и покушаш поново.",

# The composer's media notices. These are OLDER than the three features
# below — they were never in the catalogue at all, so they shipped in English
# in every language, in the same three functions as the paste strings. Six are
# word for word an Android string that was already translated, and reuse its
# Serbian exactly; the other four follow their nearest sibling.
"Couldn't download that file.": "Та датотека није преузета.",
"Couldn't download that to share.": "Преузимање ради дељења није успело.",
"Couldn't prepare that item.": "Та ставка није припремљена.",
"Couldn't read that file.": "Та датотека не може да се прочита.",
"Couldn't read that item.": "Та ставка не може да се прочита.",
"Couldn't read that video.": "Тај видео не може да се прочита.",
"Couldn't send that.": "То није послато.",
"Couldn't send that — try again.": "То није послато — покушај поново.",
"Still too large after compressing — try a shorter clip.":
  "И после компресије је превелико — пробај краћи снимак.",
"That file is over the 100 MB limit.": "Та датотека прелази ограничење од 100 MB.",

# Pasting into the composer. "Налепи" is the paste verb both platforms use;
# the two file names are what a pasted item is called once it is staged.
"Paste": "Налепи",
"Pasted image": "Налепљена слика",
"Pasted file": "Налепљена датотека",
"There's nothing to paste.": "Нема шта да се налепи.",
"Attach": "Приложи",

# Deleting your own account. Every line is a warning about something
# irreversible, so it stays plain — and GENDER-NEUTRAL, which is why none of
# it says "пријављен(а)": a session is closed, a reader is not described.
"Delete Account": "Обриши налог",
"Delete Account…": "Обриши налог…",
"Delete your account?": "Обрисати твој налог?",
"This happens immediately and cannot be undone.": "То се дешава одмах и не може да се поништи.",
"What happens": "Шта се дешава",
"Your account, password, profile picture and birthday are deleted, and every device you are signed in on is signed out.":
  "Твој налог, лозинка, слика профила и рођендан се бришу, а све сесије на свим уређајима се затварају.",
"Your direct chats are deleted — for the other person too. So is your private chat with the assistant.":
  "Твоји лични разговори се бришу — и код друге особе. Исто важи и за лични разговор са асистентом.",
"Your messages in the family chat, your board notes and your reactions stay. They are shown from then on as “Deleted account”.":
  "Твоје поруке у породичном разговору, белешке на табли и реакције остају. "
  "Од тада стоје као „Обрисан налог“.",
"You own this family: ownership passes to the longest-standing remaining member. If you are its last member, the family is deleted with you — its chat, its board and its invite code.":
  "Ова породица је твоја: прелази на члана који је у њој најдуже. Ако си њен последњи члан, "
  "породица се брише заједно са тобом — њен разговор, њена табла и њен позивни код.",
"There is no grace period and no way to cancel afterwards.":
  "Нема рока за предомишљање, нити начина да се то касније откаже.",
"Type your password to confirm it is you. Being signed in is not proof.":
  "Унеси своју лозинку да потврдиш да си то ти. Отворена сесија није доказ.",
# The three errors the sheet can show. Same wording Android already ships
# for e_wrong_password, so the two platforms word one sentence once —
# which is also why e_wrong_password is no longer in serbian_android.py.
"That password is not right.": "Та лозинка није тачна.",
"Type your password to confirm.": "Унеси лозинку да потврдиш.",
"Couldn't delete your account. Try again.": "Налог није обрисан. Покушај поново.",
"Deleted account": "Обрисан налог",

# Polls. "Одговор" for an option rather than "опција": these are the answers
# people choose between, and "опција" in Serbian reads as a setting.
"Poll": "Анкета",
"New poll": "Нова анкета",
"Create": "Направи",
"Question": "Питање",
"Ask the family something…": "Питај породицу нешто…",
"The question is the message everyone sees.": "Питање је порука коју сви виде.",
"Options": "Одговори",
"Option": "Одговор",
"Add option": "Додај одговор",
"Remove option": "Уклони одговор",
"Between 2 and 10 options. They can't be changed once the poll is sent.":
  "Између 2 и 10 одговора. После слања се више не мењају.",
# "Заврши", not "Затвори" — a poll ENDS, and "Затвори" is what a dialog does.
"Close poll": "Заврши анкету",
"Ends the poll. This cannot be undone.": "Завршава анкету. То не може да се поништи.",
"Poll closed": "Анкета завршена",
"Your choice": "Твој избор",
# "ко" always takes the masculine singular in Serbian, whoever it turns out
# to be, so this participle carries no gender — unlike "гласао/гласала" about
# the reader, which is why the counts above are a noun. Same shape as
# "Види ко је реаговао", because it is the same gesture for the same reason.
"See who voted": "Види ко је гласао",

# The composer's 4000-character ceiling (docs/protocol.md, "Limits"), said
# out loud where the text arrives instead of at Send. "знакова" is the
# genitive plural the repo already uses for a character count ("бар 8
# знакова") and the number here is always the constant 4000, so no other
# form can come up. Nothing describes the reader, so nothing is gendered:
# it is the message that is at the limit, and it is "остатак" — masculine
# by its own noun — that was not pasted.
"A message can be at most %lld characters.": "Порука може имати највише %lld знакова.",
"A message can be at most %lld characters. The rest wasn't pasted.":
  "Порука може имати највише %lld знакова. Остатак није налепљен.",
"The message is already at the %lld-character limit.":
  "Порука је већ на ограничењу од %lld знакова.",
# The two refusals a paste can hit while the composer is busy. Both are
# imperatives, which carry no gender in Serbian — unlike anything that
# would describe the person doing the pasting.
"Finish editing before attaching something.": "Заврши измену пре него што нешто приложиш.",
"Wait until the current attachment is done.": "Сачекај да се тренутни прилог заврши.",

# Opening a chat at its oldest unread message. The button was Android's
# first (s_scroll_to_newest) and the Apple clients ported its wording, so
# this moved here OUT of serbian_android.py — the rule that file exists for
# is that one sentence is worded once. The divider's count is a plural and
# lives in SR_COUNTS below.
"Scroll to newest": "Иди на најновије",

# --- voice calls (iOS + macOS; the Android twins are in serbian_android.py) ---
"%@ is calling": "%@ зове",
"Accept": "Прихвати",
"Answer": "Јави се",
"Answered on another device": "Прихваћен на другом уређају",
"Busy": "Заузето",
"Call back": "Позови назад",
"Call ended": "Позив завршен",
"Call failed · %@": "Позив није успео · %@",
"Call failed": "Позив није успео",
"Call": "Позови",
"Call %@": "Позови %@",
"Calling…": "Позивање…",
"Declined voice call": "Одбијен гласовни позив",
"Declined": "Одбијено",
"Hang Up": "Прекини",
"Incoming call": "Долазни позив",
"Microphone access is needed for calls.": "За позиве је потребан приступ микрофону.",
"Missed voice call": "Пропуштен гласовни позив",
"Mute": "Утишај",
"No answer": "Нема одговора",
"Ringing…": "Звони…",
"Speaker": "Звучник",
"Unavailable": "Недоступно",
"Unknown caller": "Непознат позивалац",
"Unmute": "Укључи звук",
"Voice call · %@": "Гласовни позив · %@",
"Voice call declined": "Гласовни позив одбијен",
"Voice call": "Гласовни позив",
}

# The members count is a plural, and Serbian's CLDR categories are one/few/other
# (1, 21, 31… / 2–4, 22–24… / everything else).
SR_PLURAL = {
    "one": "%lld члан",
    "few": "%lld члана",
    "other": "%lld чланова",
}

# A poll's counts, also one/few/other — and written ONCE here, with neutral
# placeholders, because the same Serbian has to come out four ways: an Apple
# plural variation, two Apple substitutions (the number is not the first
# argument in either) and an Android <plurals>. {n} is the number that
# inflects the noun; {total} is how many people could have voted.
#
# A NOUN ("3 гласа"), not the participle "гласало/гласао/гласала": the
# participle is gendered in the singular, and this app has no idea who is
# reading. One member casts one vote, so counting votes says exactly what
# counting voters would.
SR_COUNTS = {
    "votes": {
        "one": "{n} глас",
        "few": "{n} гласа",
        "other": "{n} гласова",
    },
    "votes_of": {
        "one": "{n} глас од {total}",
        "few": "{n} гласа од {total}",
        "other": "{n} гласова од {total}",
    },
    # The unread divider. A feminine noun, so 2–4 take the nominative
    # plural ("2 нове поруке") and everything else the genitive plural
    # ("5 нових порука"). A noun phrase again rather than a sentence, so
    # there is no participle to gender.
    "new_messages": {
        "one": "{n} нова порука",
        "few": "{n} нове поруке",
        "other": "{n} нових порука",
    },
}

# Serbian Cyrillic → Latin. A true 1:1 mapping, which is the whole reason the
# Latin set can be generated rather than translated a second time. Latin
# characters (product names like "Family Connect", "Mac") pass through
# untouched.
TRANSLIT = {
    'а':'a','б':'b','в':'v','г':'g','д':'d','ђ':'đ','е':'e','ж':'ž','з':'z',
    'и':'i','ј':'j','к':'k','л':'l','љ':'lj','м':'m','н':'n','њ':'nj','о':'o',
    'п':'p','р':'r','с':'s','т':'t','ћ':'ć','у':'u','ф':'f','х':'h','ц':'c',
    'ч':'č','џ':'dž','ш':'š',
}


def to_latin(text):
    out = []
    for i, ch in enumerate(text):
        lower = ch.lower()
        if lower not in TRANSLIT:
            out.append(ch)
            continue
        latin = TRANSLIT[lower]
        if ch == lower:
            out.append(latin)
            continue
        # An upper-case digraph is "Lj" normally and "LJ" only inside a run of
        # capitals — a sentence-case string must not shout.
        rest = text[i + 1:]
        next_ch = rest[0] if rest else ''
        if len(latin) > 1 and next_ch and next_ch.isupper():
            out.append(latin.upper())
        else:
            out.append(latin[0].upper() + latin[1:])
    return ''.join(out)
