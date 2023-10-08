// JavaScript Document

$(window).on('load', function () {
  'use strict';

  /*----------------------------------------------------*/
  /*	Preloader
		/*----------------------------------------------------*/

  $('#loader').delay(100).fadeOut();
  $('#loader-wrapper').delay(100).fadeOut('fast');

  $(window).stellar({});
});

$(window).on('scroll', function () {
  'use strict';

  /*----------------------------------------------------*/
  /*	Navigtion Menu Scroll
		/*----------------------------------------------------*/

  var b = $(window).scrollTop();

  if (b > 72) {
    $('.navbar').addClass('scroll');
  } else {
    $('.navbar').removeClass('scroll');
  }
});

/*----------------------------------------------------*/
/*	Download links
	/*----------------------------------------------------*/
var version = '1.4.1';

function getOS() {
  var userAgent = window.navigator.userAgent,
    platform = window.navigator.platform,
    macPlatforms = ['Macintosh', 'MacIntel', 'MacPPC', 'Mac68K'],
    windowsPlatforms = ['Win32', 'Win64', 'Windows', 'WinCE'],
    iosPlatforms = ['iPhone', 'iPad', 'iPod'],
    os = null;

  if (macPlatforms.indexOf(platform) !== -1) {
    os = 'Mac';
  } else if (iosPlatforms.indexOf(platform) !== -1) {
    os = 'iOS';
  } else if (windowsPlatforms.indexOf(platform) !== -1) {
    os = 'Windows';
  } else if (/Android/.test(userAgent)) {
    os = 'Android';
  } else if (!os && /Linux/.test(platform)) {
    os = 'Linux';
  }

  return os;
}

function updateDownloadLinks() {
  const baseUrl = `https://github.com/jamaljsr/polar/releases/download/v${version}`;
  const fileUrls = {
    // apple: `${baseUrl}/polar-mac-x64-v${version}.dmg`,
    // linux: `${baseUrl}/polar-linux-x86_64-v${version}.AppImage`,
    // windows: `${baseUrl}/polar-win-x64-v${version}.exe`,
       apple: 'https://github.com',
       linux: 'https://github.com',
       windows: 'https://github.com'
  };
  let primaryUrl = 'https://github.com/jamaljsr/polar/releases';
  let alt = 1;
  const detectedOS = getOS();
  Object.keys(fileUrls).forEach((os) => {
    $(`a.dl-${os}`).prop('href', fileUrls[os]);
    let osName = os[0].toUpperCase() + os.substring(1);
    if (osName === 'Apple') osName = 'Mac';
    if (detectedOS === osName) {
      primaryUrl = fileUrls[os];
      $('#hero-dl-icon').prop('class', `fab fa-${os}`);
      $('#hero-dl-text').text(`Get beta access for ${osName}`);
    } else {
      $(`.dl-alt${alt}`).text(osName).prop('href', fileUrls[os]);
      alt++;
    }
  });
  $(`a.dl-primary`).prop('href', primaryUrl);
}

$(document).ready(function () {
  'use strict';

  updateDownloadLinks();

  /*----------------------------------------------------*/
  /*	Animated Scroll To Anchor
		/*----------------------------------------------------*/

  $(
    '.header a[href^="#"], .page a.btn[href^="#"], .page a.internal-link[href^="#"]'
  ).on('click', function (e) {
    e.preventDefault();

    var target = this.hash,
      $target = jQuery(target);

    $('html, body')
      .stop()
      .animate(
        {
          scrollTop: $target.offset().top - 60, // - 200px (nav-height)
        },
        'slow',
        'easeInSine',
        function () {
          window.location.hash = '1' + target;
        }
      );
  });

  /*----------------------------------------------------*/
  /*	ScrollUp
		/*----------------------------------------------------*/

  $.scrollUp = function (options) {
    // Defaults
    var defaults = {
      scrollName: 'scrollUp', // Element ID
      topDistance: 600, // Distance from top before showing element (px)
      topSpeed: 800, // Speed back to top (ms)
      animation: 'fade', // Fade, slide, none
      animationInSpeed: 200, // Animation in speed (ms)
      animationOutSpeed: 200, // Animation out speed (ms)
      scrollText: '', // Text for element
      scrollImg: false, // Set true to use image
      activeOverlay: false, // Set CSS color to display scrollUp active point, e.g '#00FFFF'
    };

    var o = $.extend({}, defaults, options),
      scrollId = '#' + o.scrollName;

    // Create element
    $('<a/>', {
      id: o.scrollName,
      href: '#top',
      title: o.scrollText,
    }).appendTo('body');

    // If not using an image display text
    if (!o.scrollImg) {
      $(scrollId).text(o.scrollText);
    }

    // Minium CSS to make the magic happen
    $(scrollId).css({
      display: 'none',
      position: 'fixed',
      'z-index': '2147483647',
    });

    // Active point overlay
    if (o.activeOverlay) {
      $('body').append("<div id='" + o.scrollName + "-active'></div>");
      $(scrollId + '-active').css({
        position: 'absolute',
        top: o.topDistance + 'px',
        width: '100%',
        'border-top': '1px dotted ' + o.activeOverlay,
        'z-index': '2147483647',
      });
    }

    // Scroll function
    $(window).on('scroll', function () {
      switch (o.animation) {
        case 'fade':
          $(
            $(window).scrollTop() > o.topDistance
              ? $(scrollId).fadeIn(o.animationInSpeed)
              : $(scrollId).fadeOut(o.animationOutSpeed)
          );
          break;
        case 'slide':
          $(
            $(window).scrollTop() > o.topDistance
              ? $(scrollId).slideDown(o.animationInSpeed)
              : $(scrollId).slideUp(o.animationOutSpeed)
          );
          break;
        default:
          $(
            $(window).scrollTop() > o.topDistance
              ? $(scrollId).show(0)
              : $(scrollId).hide(0)
          );
      }
    });

    // To the top
    $(scrollId).on('click', function (event) {
      $('html, body').animate({ scrollTop: 0 }, o.topSpeed);
      event.preventDefault();
    });
  };

  $.scrollUp();

  /*----------------------------------------------------*/
  /*	Video Link #2 Lightbox
		/*----------------------------------------------------*/

  $('.video-popup2').magnificPopup({
    mainClass: 'video-modal',
    closeOnBgClick: false,
    type: 'iframe',
    iframe: {
      patterns: {
        youtube: {
          index: 'youtube.com',
          src: 'https://www.youtube.com/embed/mb37durvPns?autoplay=1',
        },
      },
    },
  });

  /*----------------------------------------------------*/
  /*	Statistic Counter
		/*----------------------------------------------------*/

  $('.count-element').each(function () {
    $(this).appear(
      function () {
        $(this)
          .prop('Counter', 0)
          .animate(
            {
              Counter: $(this).text(),
            },
            {
              duration: 4000,
              easing: 'swing',
              step: function (now) {
                $(this).text(Math.ceil(now));
              },
            }
          );
      },
      { accX: 0, accY: 0 }
    );
  });

  /*----------------------------------------------------*/
  /*	Testimonials Rotator
		/*----------------------------------------------------*/

  var owl = $('.reviews-holder');
  owl.owlCarousel({
    items: 3,
    loop: true,
    autoplay: true,
    navBy: 1,
    autoplayTimeout: 4500,
    autoplayHoverPause: false,
    smartSpeed: 1500,
    responsive: {
      0: {
        items: 1,
      },
      767: {
        items: 1,
      },
      768: {
        items: 2,
      },
      991: {
        items: 3,
      },
      1000: {
        items: 3,
      },
    },
  });

  /*----------------------------------------------------*/
  /*	Reviews Grid
		/*----------------------------------------------------*/

  $('.grid-loaded').imagesLoaded(function () {
    var $grid = $('.masonry-wrap').isotope({
      itemSelector: '.review-2',
      percentPosition: true,
      transitionDuration: '0.7s',
      masonry: {
        columnWidth: '.review-2',
      },
    });
  });

  /*----------------------------------------------------*/
  /*	Brands Logo Rotator
		/*----------------------------------------------------*/

  var owl = $('.brands-carousel');
  owl.owlCarousel({
    items: 6,
    loop: true,
    autoplay: true,
    navBy: 1,
    autoplayTimeout: 4000,
    autoplayHoverPause: false,
    smartSpeed: 2000,
    responsive: {
      0: {
        items: 2,
      },
      550: {
        items: 3,
      },
      767: {
        items: 3,
      },
      768: {
        items: 4,
      },
      991: {
        items: 4,
      },
      1000: {
        items: 6,
      },
    },
  });

  /*----------------------------------------------------*/
  /*	Hero Form Validation
		/*----------------------------------------------------*/

  $('.hero-form').validate({
    rules: {
      name: {
        required: true,
        minlength: 2,
        maxlength: 16,
      },
      email: {
        required: true,
        email: true,
      },
      phone: {
        required: true,
        digits: true,
      },
      subject: {
        required: true,
        minlength: 2,
      },
    },
    messages: {
      name: {
        required: 'Please enter no more than (1) characters',
      },
      email: {
        required: 'We need your email address to contact you',
        email: 'Your email address must be in the format of name@domain.com',
      },
      phone: {
        required: 'Please enter only digits',
        digits: 'Please enter a valid number',
      },
      subject: {
        required: 'Please enter no more than (1) characters',
      },
    },
  });

  /*----------------------------------------------------*/
  /*	Register Form Validation
		/*----------------------------------------------------*/

  $('.register-form').validate({
    rules: {
      name: {
        required: true,
        minlength: 2,
        maxlength: 16,
      },
      email: {
        required: true,
        email: true,
      },
    },
    messages: {
      name: {
        required: 'Please enter no more than (1) characters',
      },
      email: {
        required: 'We need your email address to contact you',
        email: 'Your email address must be in the format of name@domain.com',
      },
    },
  });

  /*----------------------------------------------------*/
  /*	Contact Form Validation
		/*----------------------------------------------------*/

  $('.contact-form').validate({
    rules: {
      name: {
        required: true,
        minlength: 1,
        maxlength: 16,
      },
      email: {
        required: true,
        email: true,
      },
      message: {
        required: true,
        minlength: 2,
      },
    },
    messages: {
      name: {
        required: 'Please enter no more than (1) characters',
      },
      email: {
        required: 'We need your email address to contact you',
        email: 'Your email address must be in the format of name@domain.com',
      },
      message: {
        required: 'Please enter no more than (2) characters',
      },
    },
  });

  /*----------------------------------------------------*/
  /*	Comment Form Validation
		/*----------------------------------------------------*/

  $('.comment-form').validate({
    rules: {
      name: {
        required: true,
        minlength: 1,
        maxlength: 16,
      },
      email: {
        required: true,
        email: true,
      },
      message: {
        required: true,
        minlength: 2,
      },
    },
    messages: {
      name: {
        required: 'Please enter no more than (1) characters',
      },
      email: {
        required: 'We need your email address to contact you',
        email: 'Your email address must be in the format of name@domain.com',
      },
      message: {
        required: 'Please enter no more than (2) characters',
      },
    },
  });

  /*----------------------------------------------------*/
  /*	Sticky Bottom Quick
		/*----------------------------------------------------*/

  $('.nb-form').hover(function () {
    $(this).toggleClass('open');
  });

  /*----------------------------------------------------*/
  /*	Newsletter Subscribe Form
		/*----------------------------------------------------*/

  $('.newsletter-form').ajaxChimp({
    language: 'cm',
    url: 'http://dsathemes.us3.list-manage.com/subscribe/post?u=af1a6c0b23340d7b339c085b4&id=344a494a6e',
    //http://xxx.xxx.list-manage.com/subscribe/post?u=xxx&id=xxx
  });

  $.ajaxChimp.translations.cm = {
    submit: 'Submitting...',
    0: 'We have sent you a confirmation email',
    1: 'Please enter your email address',
    2: 'An email address must contain a single @',
    3: 'The domain portion of the email address is invalid (the portion after the @: )',
    4: 'The username portion of the email address is invalid (the portion before the @: )',
    5: 'This email address looks fake or invalid. Please enter a real email address',
  };
});
