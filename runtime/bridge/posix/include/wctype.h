#ifndef KZ_POSIX_WCTYPE_H_
#define KZ_POSIX_WCTYPE_H_

#include <locale.h>
#include <wchar.h>

typedef uint32_t wctype_t;
typedef uint32_t wctrans_t;

#ifdef __cplusplus
extern "C" {
#endif

int iswalnum(wint_t character);
int iswalnum_l(wint_t character, locale_t locale);
int iswalpha(wint_t character);
int iswalpha_l(wint_t character, locale_t locale);
int iswblank(wint_t character);
int iswblank_l(wint_t character, locale_t locale);
int iswcntrl(wint_t character);
int iswcntrl_l(wint_t character, locale_t locale);
int iswctype(wint_t character, wctype_t character_class);
int iswctype_l(wint_t character, wctype_t character_class, locale_t locale);
int iswdigit(wint_t character);
int iswdigit_l(wint_t character, locale_t locale);
int iswgraph(wint_t character);
int iswgraph_l(wint_t character, locale_t locale);
int iswlower(wint_t character);
int iswlower_l(wint_t character, locale_t locale);
int iswprint(wint_t character);
int iswprint_l(wint_t character, locale_t locale);
int iswpunct(wint_t character);
int iswpunct_l(wint_t character, locale_t locale);
int iswspace(wint_t character);
int iswspace_l(wint_t character, locale_t locale);
int iswupper(wint_t character);
int iswupper_l(wint_t character, locale_t locale);
int iswxdigit(wint_t character);
int iswxdigit_l(wint_t character, locale_t locale);
wint_t towctrans(wint_t character, wctrans_t transformation);
wint_t towctrans_l(wint_t character, wctrans_t transformation, locale_t locale);
wint_t towlower(wint_t character);
wint_t towlower_l(wint_t character, locale_t locale);
wint_t towupper(wint_t character);
wint_t towupper_l(wint_t character, locale_t locale);
wctrans_t wctrans(const char *name);
wctrans_t wctrans_l(const char *name, locale_t locale);
wctype_t wctype(const char *name);
wctype_t wctype_l(const char *name, locale_t locale);

#ifdef __cplusplus
}
#endif

#endif