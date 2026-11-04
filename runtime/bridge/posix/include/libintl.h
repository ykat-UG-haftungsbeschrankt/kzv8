#ifndef KZ_POSIX_LIBINTL_H_
#define KZ_POSIX_LIBINTL_H_

#include <locale.h>

#ifdef __cplusplus
extern "C" {
#endif

char *bindtextdomain(const char *domain, const char *directory);
char *bind_textdomain_codeset(const char *domain, const char *codeset);
char *dcgettext(const char *domain, const char *message, int category);
char *dcgettext_l(const char *domain, const char *message, int category,
                  locale_t locale);
char *dcngettext(const char *domain, const char *singular,
                 const char *plural, unsigned long count, int category);
char *dcngettext_l(const char *domain, const char *singular,
                   const char *plural, unsigned long count, int category,
                   locale_t locale);
char *dgettext(const char *domain, const char *message);
char *dgettext_l(const char *domain, const char *message, locale_t locale);
char *dngettext(const char *domain, const char *singular,
                const char *plural, unsigned long count);
char *dngettext_l(const char *domain, const char *singular,
                  const char *plural, unsigned long count, locale_t locale);
char *gettext(const char *message);
char *gettext_l(const char *message, locale_t locale);
char *ngettext(const char *singular, const char *plural, unsigned long count);
char *ngettext_l(const char *singular, const char *plural,
                 unsigned long count, locale_t locale);
char *textdomain(const char *domain);

#ifdef __cplusplus
}
#endif

#endif