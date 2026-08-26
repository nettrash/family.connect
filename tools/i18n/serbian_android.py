# -*- coding: utf-8 -*-
"""The Android strings that have no iOS counterpart to reuse.

Everything whose English matches an iOS key (exactly, or bar capitalisation)
is taken from serbian.SR instead — the two platforms must not word the same
sentence differently.
"""

ANDROID_ONLY = {
# Birthdays and the assistant's family settings. The four Android words the
# iOS catalogue already has — Birthday, Not set, Month, Day — are found by
# the English match and are deliberately NOT repeated here.
"s_family_language": "Језик породице",
"s_family_language_explanation":
  "Језик на ком асистент одговара када га питаш у породичном разговору. "
  "Језик саме апликације се тиме не мења. "
  "Ако ништа није изабрано, одговор стиже на језику питања.",
"s_assistant_history": "Асистент види недавну историју",
"s_assistant_history_explanation":
  "Када неко помене асистента у породичном разговору, уз питање се шаље и преписка "
  "из последњих месец дана, па може да одговори и на оно што је раније речено. "
  "Када је искључено, шаље се само порука у којој се помиње.",
"s_birthday_for": "Рођендан за %1$s",
"s_day_and_month_no_year": "Дан и месец. Нема године, па ни узраста.",
"e_birthday_failed": "Рођендан није сачуван",
"e_change_language_failed": "Језик породице није промењен",
"e_change_assistant_history_failed": "Подешавање није промењено",

# The statistics strings whose English differs from iOS only in the format
# specifiers (%1$d/%2$s rather than %lld/%@), so the English match cannot find
# them. Same sentence, same wording as ios/FamilyConnect/Views/StatisticsView.
"s_saved_by_one_copy": "%1$s уштеђено — исте датотеке се чувају у једном примерку.",
"s_attachments_and_size": "прилога: %1$d, %2$s",
"s_questions_to_assistant": "питања асистенту: %1$d",
"s_record_audio": "Сними аудио",
"s_stop": "Заустави",
"s_audio": "Аудио",
"s_play": "Пусти",
"s_pause": "Пауза",
"e_record_failed": "Снимање није могло да почне.",
"e_recording_too_short": "Снимак је прекратак.",
"e_microphone_permission": "Family треба дозволу за коришћење микрофона.",
"s_ask_the_assistant": "Питај асистента",
"s_map_previews": "Приказ мапе",
"s_map_previews_explanation": "Приказује мапу уз подељену локацију. Мапа се тражи од Google-а, па Google види захтев са овог уређаја. Ако је искључено, локација и даље приказује ознаку и отвара се у твојој апликацији за мапе на додир.",
"s_location": "Локација",
"s_share_your_location": "Локација",
"e_location_permission": "Family треба дозволу за приступ твојој локацији.",
"e_location_unavailable": "Локација није могла да се одреди.",
"s_take_photo": "Сликај",
"s_record_video": "Сними видео",
"s_add_a_message_or_send_it_on_its_own": "Додај поруку или пошаљи овако.",
"s_remove_attachment": "Уклони прилог",
"s_approval": "Одобрење",
"s_back": "Назад",
"s_chat_example_com_or_http_192_168_1_10_8080": "chat.example.com или http://192.168.1.10:8080",
"s_check_again": "Провери поново",
"s_copy_invite_code": "Копирај позивни код",
"s_failed_tap_to_retry": "Није успело — додирни да покушаш поново",
"s_join_with_an_invite_code": "Придружи се позивним кодом",
"s_leave": "Напусти",
"s_message_not_sent": "Порука није послата",
"s_new_password": "Нова лозинка",
"s_no_other_family_members_yet_share_the_invite_code_first":
  "Још нема других чланова породице — прво подели позивни код.",
"s_open": "Отвори",
"s_open_link": "Отвори линк",
"s_register": "Региструј се",
"s_request_declined": "Захтев је одбијен",
"s_rotate_invite_code": "Промени позивни код",
"s_rotating_invalidates_the_current_code_immediately": "Тренутни код одмах престаје да важи",
"s_save_anyway": "Ипак сачувај",
"s_save_to_gallery": "Сачувај у галерију",
"s_share_invite_code": "Подели позивни код",
"s_share_it_to_invite_family_members": "Подели га да позовеш чланове породице",
"s_share_the_invite_code_below_to_add_someone": "Подели позивни код испод да додаш некога.",
"s_start_a_new_family_space_or_join_one_you_were_invited_to":
  "Направи нови породични простор или се придружи оном за који имаш позив.",
"s_the_family_chat_appears_as_soon_as_you_re_connected":
  "Породични разговор се појављује чим се повежеш.",
"s_the_server_may_be_offline_right_now": "Сервер је можда тренутно недоступан",
"s_they_lose_access_to_the_family_chats_history_stays_and_ret":
  "Губе приступ породичним разговорима. Историја остаје и враћа се ако се поново придруже.",
"s_try_sending_it_again_or_delete_the_draft": "Покушај да пошаљеш поново или обриши нацрт.",
"s_use_a_different_server": "Користи други сервер",
"s_welcome": "Здраво",
"s_where_does_your_family_s_server_live": "Где се налази сервер твоје породице?",
"s_copied": "Копирано",
"s_saved_to_gallery": "Сачувано у галерију",
"s_leave_family_explanation":
  "Изгубићеш приступ породичном разговору и својим личним разговорима. "
  "Историја остаје на серверу и враћа се ако се поново придружиш.",
"s_plain_http_warning":
  "Ова адреса користи обичан http — поруке путују мрежом нешифроване. "
  "У поузданој кућној мрежи је у реду, било где другде је ризично.",
"s_request_declined_explanation":
  "Власник породице је одбио твој захтев. Можеш да пробаш други позивни код "
  "или да направиш своју породицу.",
"s_waiting_explanation":
  "Одавде нема шта да се откаже — власник ће те или пустити унутра или одбити захтев.",

"e_action_failed": "Радња није успела",
"e_already_in_family": "Већ си у породици — повуци да освежиш",
"e_change_password_failed": "Лозинка није промењена.",
"e_change_policy_failed": "Правило није промењено",
"e_create_family_failed": "Породица није направљена",
"e_download_to_save_failed": "Преузимање ради чувања није успело.",
"e_enter_invite_code": "Унеси позивни код",
"e_gallery_permission": "Family треба дозволу да чува у твоју галерију.",
"e_image_unreadable": "Та слика не може да се прочита.",
"e_invalid_address": "То не личи на исправну адресу",
"e_invalid_invite_code": "Неисправан позивни код",
"e_join_failed": "Придруживање није успело",
"e_load_requests_failed": "Захтеви за приступ нису учитани",
"e_no_app_for_file": "Ниједна апликација на овом телефону не може да отвори ту датотеку.",
"e_password_too_short": "Најмање 8 знакова",
"e_rotate_failed": "Промена кода није успела",
"e_save_failed": "То није сачувано.",
"e_unreachable": "Сервер није доступан",
"e_unreachable_check_connection": "Сервер није доступан — провери везу",
"e_username_taken": "Корисничко име је заузето",
"e_wrong_credentials": "Погрешно корисничко име или лозинка",
"e_wrong_current_password": "Тренутна лозинка није тачна.",

# Pasting: the only word Android has that iOS does not. An image and a file
# are found by the English match.
"s_pasted_sound": "Налепљени звук",

# The composer's 4000-character ceiling. The two sentences are word for word
# the iOS ones, but the English cannot match: Android writes %1$d where the
# catalogue writes %lld, the same reason the statistics strings above are
# here. e_finish_editing_first is genuinely different — Android names the
# message ("this message"), because its refusal replaces the media strip's
# own text rather than sitting under the edit banner.
"e_message_at_limit": "Порука је већ на ограничењу од %1$d знакова.",
"e_paste_truncated": "Порука може имати највише %1$d знакова. Остатак није налепљен.",
"e_finish_editing_first": "Прво заврши измену ове поруке.",

# Deleting your own account. Android says in one paragraph what iOS says in
# three separate lines, so the wording cannot be shared — but it must say the
# same things, in the same plain and GENDER-NEUTRAL Serbian.
"s_delete_account_explanation":
  "Твој налог, лозинка, слика профила и рођендан се бришу, а све сесије на свим уређајима "
  "се затварају. Бришу се и лични разговори — и код друге особе. Остаје оно што сте једни "
  "другима рекли: твоје поруке у породичном разговору, белешке на табли и реакције, које "
  "од тада стоје као „Обрисан налог“. То не може да се поништи.",
"s_delete_account_owner_note":
  "Ова породица је твоја, па прелази на онога ко је у њој најдуже. Ако си њен последњи "
  "члан, породица се брише заједно са тобом — њен разговор, њена табла и њен позивни код.",
# e_wrong_password is NOT here: iOS now carries the same English sentence,
# so the generator's English match resolves it — the rule this file exists
# to keep. e_delete_account_failed stays, because Android's wording ends
# there where iOS adds "Try again."
"e_delete_account_failed": "Налог није обрисан.",

# Polls. The words themselves come from the iOS catalogue by the English
# match; only this refusal has no iOS counterpart.
"e_close_poll_failed": "Анкета није завршена.",
# The who-voted list behind an option's faces. Here rather than found by
# the English match because iOS may word its own overflow differently; if
# the catalogue ever gains this exact English, move it to serbian.SR and
# delete this line, so the two platforms cannot drift apart.
"s_who_voted": "Ко је гласао",
}
